use std::sync::{Arc, RwLock};

use crate::catalog;
use crate::core::error::{RoutingError, RuntimeError};
use crate::core::traits::ProviderAdapter;
use crate::core::types::{AdapterContext, DiscoveryOptions, ModelCatalog, ModelRef, ProviderId};

pub struct ProviderRegistry {
    adapters: Vec<(ProviderId, Arc<dyn ProviderAdapter>)>,
    static_catalog: ModelCatalog,
    active_catalog: RwLock<ModelCatalog>,
    default_provider: Option<ProviderId>,
}

impl ProviderRegistry {
    pub fn new(static_catalog: ModelCatalog, default_provider: Option<ProviderId>) -> Self {
        Self {
            adapters: Vec::new(),
            active_catalog: RwLock::new(static_catalog.clone()),
            static_catalog,
            default_provider,
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        let provider = adapter.id();

        if let Some((_, existing_adapter)) = self
            .adapters
            .iter_mut()
            .find(|(registered_provider, _)| *registered_provider == provider)
        {
            *existing_adapter = adapter;
            return;
        }

        self.adapters.push((provider, adapter));
    }

    pub fn resolve_adapter(
        &self,
        provider: &ProviderId,
    ) -> Result<Arc<dyn ProviderAdapter>, RoutingError> {
        self.adapters
            .iter()
            .find(|(registered_provider, _)| registered_provider == provider)
            .map(|(_, adapter)| Arc::clone(adapter))
            .ok_or_else(|| RoutingError::ProviderNotRegistered {
                provider: provider.clone(),
            })
    }

    pub fn resolve_provider(&self, model: &ModelRef) -> Result<ProviderId, RoutingError> {
        if let Some(provider_hint) = &model.provider_hint {
            self.resolve_adapter(provider_hint)?;
            return Ok(provider_hint.clone());
        }

        let active_catalog = self.read_active_catalog();
        match catalog::resolve_model_provider(&active_catalog, &model.model_id, None) {
            Ok(provider) => {
                self.resolve_adapter(&provider)?;
                Ok(provider)
            }
            Err(RoutingError::ModelNotFound { model }) => {
                if let Some(default_provider) = &self.default_provider {
                    self.resolve_adapter(default_provider)?;
                    return Ok(default_provider.clone());
                }

                Err(RoutingError::ModelNotFound { model })
            }
            Err(error) => Err(error),
        }
    }

    pub async fn discover_models(
        &self,
        opts: &DiscoveryOptions,
        ctx: &AdapterContext,
    ) -> Result<ModelCatalog, RuntimeError> {
        if !opts.remote || !opts.refresh_cache {
            return Ok(self.read_active_catalog());
        }

        let mut adapters = self
            .adapters
            .iter()
            .map(|(provider, adapter)| (provider.clone(), Arc::clone(adapter)))
            .collect::<Vec<_>>();
        adapters.sort_by_key(|(provider, _)| provider_sort_key(provider));

        let mut remote_models = Vec::new();
        for (provider, adapter) in adapters {
            if !opts.include_provider.is_empty() && !opts.include_provider.contains(&provider) {
                continue;
            }

            if !adapter.capabilities().supports_remote_discovery {
                continue;
            }

            let discovered = adapter.discover_models(opts, ctx).await?;
            remote_models.extend(discovered);
        }

        let merged_catalog = catalog::merge_static_and_remote_catalog(
            &self.static_catalog,
            &ModelCatalog {
                models: remote_models,
            },
        );

        self.write_active_catalog(merged_catalog.clone());

        Ok(merged_catalog)
    }

    fn read_active_catalog(&self) -> ModelCatalog {
        self.active_catalog
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn write_active_catalog(&self, catalog: ModelCatalog) {
        *self
            .active_catalog
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = catalog;
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new(catalog::builtin_static_catalog(), None)
    }
}

fn provider_sort_key(provider: &ProviderId) -> u8 {
    match provider {
        ProviderId::Openai => 0,
        ProviderId::Anthropic => 1,
        ProviderId::Openrouter => 2,
        ProviderId::Custom => 3,
    }
}

#[cfg(test)]
mod tests;
