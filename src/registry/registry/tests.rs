use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::ProviderRegistry;
use crate::core::error::{ProviderError, RoutingError};
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, AssistantOutput, ContentPart, DiscoveryOptions, FinishReason, ModelCatalog,
    ModelInfo, ModelRef, ProviderCapabilities, ProviderId, ProviderRequest, ProviderResponse,
    ToolChoice, Usage,
};

#[derive(Clone)]
struct MockAdapter {
    provider: ProviderId,
    capabilities: ProviderCapabilities,
    discovered_models: Vec<ModelInfo>,
    discover_calls: Arc<Mutex<u32>>,
}

impl MockAdapter {
    fn new(
        provider: ProviderId,
        capabilities: ProviderCapabilities,
        discovered_models: Vec<ModelInfo>,
    ) -> Self {
        Self {
            provider,
            capabilities,
            discovered_models,
            discover_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn discover_call_count(&self) -> u32 {
        *self
            .discover_calls
            .lock()
            .expect("discover_calls lock should not be poisoned")
    }
}

#[async_trait]
impl ProviderAdapter for MockAdapter {
    fn id(&self) -> ProviderId {
        self.provider.clone()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }

    async fn run(
        &self,
        req: &ProviderRequest,
        _ctx: &AdapterContext,
    ) -> Result<ProviderResponse, ProviderError> {
        Ok(ProviderResponse {
            output: AssistantOutput {
                content: vec![ContentPart::Text {
                    text: "ok".to_string(),
                }],
                structured_output: None,
            },
            usage: Usage::default(),
            cost: None,
            provider: self.provider.clone(),
            model: req.model.model_id.clone(),
            raw_provider_response: None,
            finish_reason: FinishReason::Stop,
            warnings: Vec::new(),
        })
    }

    async fn discover_models(
        &self,
        _opts: &DiscoveryOptions,
        _ctx: &AdapterContext,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        *self
            .discover_calls
            .lock()
            .expect("discover_calls lock should not be poisoned") += 1;
        Ok(self.discovered_models.clone())
    }
}

fn adapter_with_models(
    provider: ProviderId,
    supports_remote_discovery: bool,
    discovered_models: Vec<ModelInfo>,
) -> MockAdapter {
    MockAdapter::new(
        provider,
        ProviderCapabilities {
            supports_tools: true,
            supports_structured_output: true,
            supports_thinking: false,
            supports_remote_discovery,
        },
        discovered_models,
    )
}

fn model(
    provider: ProviderId,
    model_id: &str,
    display_name: Option<&str>,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
) -> ModelInfo {
    ModelInfo {
        provider,
        model_id: model_id.to_string(),
        display_name: display_name.map(ToString::to_string),
        context_window,
        max_output_tokens,
        supports_tools: true,
        supports_structured_output: true,
    }
}

fn model_ref(model_id: &str, provider_hint: Option<ProviderId>) -> ModelRef {
    ModelRef {
        provider_hint,
        model_id: model_id.to_string(),
    }
}

fn discover_opts(
    remote: bool,
    refresh_cache: bool,
    include_provider: Vec<ProviderId>,
) -> DiscoveryOptions {
    DiscoveryOptions {
        remote,
        include_provider,
        refresh_cache,
    }
}

#[test]
fn test_registry_register_and_lookup() {
    let mut registry = ProviderRegistry::new(ModelCatalog::default(), None);

    let first = adapter_with_models(ProviderId::Openai, true, Vec::new());
    registry.register(Arc::new(first));

    let resolved = registry
        .resolve_adapter(&ProviderId::Openai)
        .expect("registered provider should resolve");
    assert_eq!(resolved.id(), ProviderId::Openai);
    assert!(resolved.capabilities().supports_remote_discovery);

    let replacement = MockAdapter::new(
        ProviderId::Openai,
        ProviderCapabilities {
            supports_tools: true,
            supports_structured_output: true,
            supports_thinking: false,
            supports_remote_discovery: false,
        },
        Vec::new(),
    );
    registry.register(Arc::new(replacement));

    let resolved_after_replace = registry
        .resolve_adapter(&ProviderId::Openai)
        .expect("replacement adapter should resolve");
    assert!(
        !resolved_after_replace
            .capabilities()
            .supports_remote_discovery
    );

    let missing = match registry.resolve_adapter(&ProviderId::Anthropic) {
        Ok(_) => panic!("unregistered provider should fail"),
        Err(error) => error,
    };
    assert_eq!(
        missing,
        RoutingError::ProviderNotRegistered {
            provider: ProviderId::Anthropic
        }
    );
}

#[test]
fn test_routing_precedence_order() {
    let static_catalog = ModelCatalog {
        models: vec![model(
            ProviderId::Anthropic,
            "shared-catalog-model",
            Some("catalog"),
            None,
            None,
        )],
    };
    let mut registry = ProviderRegistry::new(static_catalog, Some(ProviderId::Openrouter));
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Openai,
        true,
        Vec::new(),
    )));
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Anthropic,
        true,
        Vec::new(),
    )));
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Openrouter,
        true,
        Vec::new(),
    )));

    let hint_wins = registry
        .resolve_provider(&model_ref("shared-catalog-model", Some(ProviderId::Openai)))
        .expect("provider hint should win when adapter exists");
    assert_eq!(hint_wins, ProviderId::Openai);

    let catalog_wins = registry
        .resolve_provider(&model_ref("shared-catalog-model", None))
        .expect("catalog route should resolve");
    assert_eq!(catalog_wins, ProviderId::Anthropic);

    let default_wins = registry
        .resolve_provider(&model_ref("unknown-model", None))
        .expect("default provider should be used for missing model");
    assert_eq!(default_wins, ProviderId::Openrouter);
}

#[test]
fn test_ambiguous_model_returns_error() {
    let static_catalog = ModelCatalog {
        models: vec![
            model(ProviderId::Openai, "shared-model", None, None, None),
            model(ProviderId::Anthropic, "shared-model", None, None, None),
        ],
    };
    let mut registry = ProviderRegistry::new(static_catalog, None);
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Openai,
        true,
        Vec::new(),
    )));
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Anthropic,
        true,
        Vec::new(),
    )));

    let err = registry
        .resolve_provider(&model_ref("shared-model", None))
        .expect_err("ambiguous model should error");
    assert_eq!(
        err,
        RoutingError::AmbiguousModelRoute {
            model: "shared-model".to_string(),
            candidates: vec![ProviderId::Openai, ProviderId::Anthropic],
        }
    );
}

#[test]
fn test_resolve_provider_uses_default_for_unknown_model_when_no_hint() {
    let mut registry = ProviderRegistry::new(ModelCatalog::default(), Some(ProviderId::Openrouter));
    registry.register(Arc::new(adapter_with_models(
        ProviderId::Openrouter,
        true,
        Vec::new(),
    )));

    let resolved = registry
        .resolve_provider(&model_ref("does-not-exist", None))
        .expect("default provider should resolve unknown model");
    assert_eq!(resolved, ProviderId::Openrouter);
}

#[test]
fn test_resolve_provider_hint_requires_registered_adapter() {
    let registry = ProviderRegistry::new(ModelCatalog::default(), None);

    let err = registry
        .resolve_provider(&model_ref("any-model", Some(ProviderId::Openai)))
        .expect_err("provider hint requires a registered adapter");
    assert_eq!(
        err,
        RoutingError::ProviderNotRegistered {
            provider: ProviderId::Openai
        }
    );
}

#[tokio::test]
async fn test_discover_models_remote_refresh_merges_static_first() {
    let static_catalog = ModelCatalog {
        models: vec![model(
            ProviderId::Openai,
            "gpt-5-mini",
            Some("Static GPT"),
            Some(128_000),
            None,
        )],
    };
    let mut registry = ProviderRegistry::new(static_catalog, None);

    let openai = adapter_with_models(
        ProviderId::Openai,
        true,
        vec![model(
            ProviderId::Openai,
            "gpt-5-mini",
            Some("Remote GPT"),
            Some(256_000),
            Some(16_000),
        )],
    );
    let anthropic = adapter_with_models(
        ProviderId::Anthropic,
        true,
        vec![model(
            ProviderId::Anthropic,
            "claude-3-7-sonnet",
            Some("Claude"),
            Some(200_000),
            Some(8_000),
        )],
    );
    registry.register(Arc::new(openai.clone()));
    registry.register(Arc::new(anthropic.clone()));

    let discovered = registry
        .discover_models(
            &discover_opts(true, true, Vec::new()),
            &AdapterContext::default(),
        )
        .await
        .expect("remote refresh should succeed");

    assert_eq!(openai.discover_call_count(), 1);
    assert_eq!(anthropic.discover_call_count(), 1);
    assert_eq!(discovered.models.len(), 2);
    assert_eq!(discovered.models[0].provider, ProviderId::Openai);
    assert_eq!(discovered.models[0].model_id, "gpt-5-mini");
    assert_eq!(
        discovered.models[0].display_name.as_deref(),
        Some("Static GPT")
    );
    assert_eq!(discovered.models[0].context_window, Some(128_000));
    assert_eq!(discovered.models[0].max_output_tokens, Some(16_000));
    assert_eq!(discovered.models[1].provider, ProviderId::Anthropic);
    assert_eq!(discovered.models[1].model_id, "claude-3-7-sonnet");
}

#[tokio::test]
async fn test_discover_models_respects_include_provider_filter() {
    let mut registry = ProviderRegistry::new(ModelCatalog::default(), None);
    let openai = adapter_with_models(
        ProviderId::Openai,
        true,
        vec![model(ProviderId::Openai, "gpt-5-mini", None, None, None)],
    );
    let anthropic = adapter_with_models(
        ProviderId::Anthropic,
        true,
        vec![model(
            ProviderId::Anthropic,
            "claude-3-7-sonnet",
            None,
            None,
            None,
        )],
    );
    registry.register(Arc::new(openai.clone()));
    registry.register(Arc::new(anthropic.clone()));

    let discovered = registry
        .discover_models(
            &discover_opts(true, true, vec![ProviderId::Anthropic]),
            &AdapterContext::default(),
        )
        .await
        .expect("filtered remote discovery should succeed");

    assert_eq!(openai.discover_call_count(), 0);
    assert_eq!(anthropic.discover_call_count(), 1);
    assert_eq!(discovered.models.len(), 1);
    assert_eq!(discovered.models[0].provider, ProviderId::Anthropic);
    assert_eq!(discovered.models[0].model_id, "claude-3-7-sonnet");
}

#[tokio::test]
async fn test_discover_models_remote_false_returns_cached_catalog() {
    let mut registry = ProviderRegistry::new(ModelCatalog::default(), None);
    let openai = adapter_with_models(
        ProviderId::Openai,
        true,
        vec![model(ProviderId::Openai, "gpt-5-mini", None, None, None)],
    );
    registry.register(Arc::new(openai.clone()));

    let refreshed = registry
        .discover_models(
            &discover_opts(true, true, Vec::new()),
            &AdapterContext::default(),
        )
        .await
        .expect("refresh should succeed");
    assert_eq!(openai.discover_call_count(), 1);

    let cached = registry
        .discover_models(
            &discover_opts(false, false, Vec::new()),
            &AdapterContext::default(),
        )
        .await
        .expect("non-remote discovery should return cache");

    assert_eq!(openai.discover_call_count(), 1);
    assert_eq!(cached, refreshed);
}

#[test]
fn test_resolve_provider_default_requires_registered_adapter() {
    let registry = ProviderRegistry::new(ModelCatalog::default(), Some(ProviderId::Openrouter));

    let err = registry
        .resolve_provider(&model_ref("unknown", None))
        .expect_err("default provider must be registered");
    assert_eq!(
        err,
        RoutingError::ProviderNotRegistered {
            provider: ProviderId::Openrouter
        }
    );
}

#[test]
fn test_resolve_provider_catalog_route_requires_registered_adapter() {
    let static_catalog = ModelCatalog {
        models: vec![model(ProviderId::Openai, "gpt-5-mini", None, None, None)],
    };
    let registry = ProviderRegistry::new(static_catalog, None);

    let err = registry
        .resolve_provider(&model_ref("gpt-5-mini", None))
        .expect_err("catalog route requires registered adapter");
    assert_eq!(
        err,
        RoutingError::ProviderNotRegistered {
            provider: ProviderId::Openai
        }
    );
}

#[tokio::test]
async fn test_default_registry_uses_builtin_catalog() {
    let registry = ProviderRegistry::default();
    let catalog = registry
        .discover_models(
            &discover_opts(false, false, Vec::new()),
            &AdapterContext::default(),
        )
        .await
        .expect("default catalog discovery should work");

    assert!(!catalog.models.is_empty());
}

#[tokio::test]
async fn test_mock_adapter_run_smoke() {
    let adapter = adapter_with_models(ProviderId::Openai, true, Vec::new());
    let request = ProviderRequest {
        model: model_ref("gpt-5-mini", Some(ProviderId::Openai)),
        messages: Vec::new(),
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        response_format: Default::default(),
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stop: Vec::new(),
        metadata: Default::default(),
    };
    let response = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect("mock adapter run should succeed");
    assert_eq!(response.provider, ProviderId::Openai);
    assert_eq!(response.model, "gpt-5-mini");
}
