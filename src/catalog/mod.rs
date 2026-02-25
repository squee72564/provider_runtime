use crate::core::error::{RoutingError, RuntimeError};
use crate::core::types::{ModelCatalog, ModelInfo, ProviderId};

pub fn merge_static_and_remote_catalog(
    static_catalog: &ModelCatalog,
    remote_catalog: &ModelCatalog,
) -> ModelCatalog {
    let mut merged = Vec::new();
    let mut static_keys = Vec::new();
    let mut remote_keys = Vec::new();

    for model in &static_catalog.models {
        let key = model_key(model);
        if contains_key(&static_keys, &key) {
            continue;
        }

        static_keys.push(key);
        merged.push(model.clone());
    }

    for model in &remote_catalog.models {
        let key = model_key(model);
        if contains_key(&remote_keys, &key) {
            continue;
        }
        remote_keys.push(key.clone());

        if let Some(index) = find_model_index(&merged, &model.provider, &model.model_id) {
            fill_missing_optional_metadata(&mut merged[index], model);
        } else {
            merged.push(model.clone());
        }
    }

    sort_models(&mut merged);

    ModelCatalog { models: merged }
}

pub fn resolve_model_provider(
    catalog: &ModelCatalog,
    model_id: &str,
    provider_hint: Option<ProviderId>,
) -> Result<ProviderId, RoutingError> {
    let mut candidates = unique_providers_for_model(catalog, model_id);

    if candidates.is_empty() {
        return Err(RoutingError::ModelNotFound {
            model: model_id.to_string(),
        });
    }

    sort_providers(&mut candidates);

    if let Some(hint) = provider_hint {
        if candidates.contains(&hint) {
            return Ok(hint);
        }

        if candidates.len() == 1 {
            return Err(RoutingError::ProviderHintMismatch {
                model: model_id.to_string(),
                provider_hint: hint,
                resolved: candidates[0].clone(),
            });
        }

        return Err(RoutingError::AmbiguousModelRoute {
            model: model_id.to_string(),
            candidates,
        });
    }

    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }

    Err(RoutingError::AmbiguousModelRoute {
        model: model_id.to_string(),
        candidates,
    })
}

pub fn export_catalog_json(catalog: &ModelCatalog) -> Result<String, RuntimeError> {
    let mut normalized = catalog.clone();
    sort_models(&mut normalized.models);

    serde_json::to_string_pretty(&normalized).map_err(|error| RuntimeError::SerializationError {
        provider: None,
        model: None,
        request_id: None,
        message: error.to_string(),
    })
}

pub(crate) fn builtin_static_catalog() -> ModelCatalog {
    ModelCatalog {
        models: vec![
            ModelInfo {
                provider: ProviderId::Openai,
                model_id: "gpt-5-mini".to_string(),
                display_name: Some("GPT-5 Mini".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
            ModelInfo {
                provider: ProviderId::Anthropic,
                model_id: "claude-3-7-sonnet".to_string(),
                display_name: Some("Claude 3.7 Sonnet".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
            ModelInfo {
                provider: ProviderId::Openrouter,
                model_id: "openrouter/auto".to_string(),
                display_name: Some("OpenRouter Auto".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
        ],
    }
}

fn model_key(model: &ModelInfo) -> (ProviderId, &str) {
    (model.provider.clone(), model.model_id.as_str())
}

fn contains_key(keys: &[(ProviderId, &str)], key: &(ProviderId, &str)) -> bool {
    keys.contains(key)
}

fn find_model_index(models: &[ModelInfo], provider: &ProviderId, model_id: &str) -> Option<usize> {
    models
        .iter()
        .position(|candidate| candidate.provider == *provider && candidate.model_id == model_id)
}

fn fill_missing_optional_metadata(target: &mut ModelInfo, source: &ModelInfo) {
    if target.display_name.is_none() {
        target.display_name = source.display_name.clone();
    }

    if target.context_window.is_none() {
        target.context_window = source.context_window;
    }

    if target.max_output_tokens.is_none() {
        target.max_output_tokens = source.max_output_tokens;
    }
}

fn unique_providers_for_model(catalog: &ModelCatalog, model_id: &str) -> Vec<ProviderId> {
    let mut providers = Vec::new();

    for model in &catalog.models {
        if model.model_id != model_id {
            continue;
        }

        if !providers.contains(&model.provider) {
            providers.push(model.provider.clone());
        }
    }

    providers
}

fn sort_models(models: &mut [ModelInfo]) {
    models.sort_by(|left, right| {
        provider_order(&left.provider)
            .cmp(&provider_order(&right.provider))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
}

fn sort_providers(providers: &mut [ProviderId]) {
    providers.sort_by_key(provider_order);
}

fn provider_order(provider: &ProviderId) -> u8 {
    match provider {
        ProviderId::Openai => 0,
        ProviderId::Anthropic => 1,
        ProviderId::Openrouter => 2,
        ProviderId::Other(_) => 3,
    }
}

#[cfg(test)]
mod tests;
