use std::sync::Arc;

use crate::catalog;
use crate::core::error::RuntimeError;
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, DiscoveryOptions, ModelCatalog, ProviderId, ProviderRequest, ProviderResponse,
    ResponseFormat,
};
use crate::pricing::{self, PricingTable};
use crate::registry::registry::ProviderRegistry;

pub struct ProviderRuntime {
    registry: ProviderRegistry,
    adapter_context: AdapterContext,
    pricing_table: Option<PricingTable>,
}

pub struct ProviderRuntimeBuilder {
    adapters: Vec<Arc<dyn ProviderAdapter>>,
    static_catalog: ModelCatalog,
    default_provider: Option<ProviderId>,
    pricing_table: Option<PricingTable>,
    adapter_context: AdapterContext,
}

impl ProviderRuntime {
    pub fn builder() -> ProviderRuntimeBuilder {
        ProviderRuntimeBuilder {
            adapters: Vec::new(),
            static_catalog: catalog::builtin_static_catalog(),
            default_provider: None,
            pricing_table: None,
            adapter_context: AdapterContext::default(),
        }
    }

    pub async fn run(&self, request: ProviderRequest) -> Result<ProviderResponse, RuntimeError> {
        let provider = self.registry.resolve_provider(&request.model)?;
        let adapter = self.registry.resolve_adapter(&provider)?;
        let capabilities = adapter.capabilities();

        if !request.tools.is_empty() && !capabilities.supports_tools {
            return Err(RuntimeError::CapabilityMismatch {
                provider,
                model: request.model.model_id,
                capability: "tools".to_string(),
            });
        }

        if !matches!(request.response_format, ResponseFormat::Text)
            && !capabilities.supports_structured_output
        {
            return Err(RuntimeError::CapabilityMismatch {
                provider,
                model: request.model.model_id,
                capability: "structured_output".to_string(),
            });
        }

        let mut response = adapter.run(&request, &self.adapter_context).await?;

        if response.cost.is_none() {
            if let Some(pricing_table) = &self.pricing_table {
                let (cost, warnings) = pricing::estimate_cost(
                    &response.provider,
                    &response.model,
                    &response.usage,
                    pricing_table,
                );
                response.cost = cost;
                response.warnings.extend(warnings);
            }
        }

        Ok(response)
    }

    pub async fn discover_models(
        &self,
        opts: DiscoveryOptions,
    ) -> Result<ModelCatalog, RuntimeError> {
        self.registry
            .discover_models(&opts, &self.adapter_context)
            .await
    }

    pub fn export_catalog_json(&self, catalog: &ModelCatalog) -> Result<String, RuntimeError> {
        catalog::export_catalog_json(catalog)
    }
}

impl ProviderRuntimeBuilder {
    pub fn with_adapter(mut self, adapter: Arc<dyn ProviderAdapter>) -> Self {
        self.adapters.push(adapter);
        self
    }

    pub fn with_default_provider(mut self, provider: ProviderId) -> Self {
        self.default_provider = Some(provider);
        self
    }

    pub fn with_model_catalog(mut self, catalog: ModelCatalog) -> Self {
        self.static_catalog = catalog;
        self
    }

    pub fn with_pricing_table(mut self, pricing_table: PricingTable) -> Self {
        self.pricing_table = Some(pricing_table);
        self
    }

    pub fn with_adapter_context(mut self, adapter_context: AdapterContext) -> Self {
        self.adapter_context = adapter_context;
        self
    }

    pub fn build(self) -> ProviderRuntime {
        let mut registry = ProviderRegistry::new(self.static_catalog, self.default_provider);
        for adapter in self.adapters {
            registry.register(adapter);
        }

        ProviderRuntime {
            registry,
            adapter_context: self.adapter_context,
            pricing_table: self.pricing_table,
        }
    }
}

#[cfg(test)]
mod tests;
