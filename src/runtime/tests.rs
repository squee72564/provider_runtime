use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::ProviderRuntime;
use crate::core::error::ProviderError;
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, AssistantOutput, ContentPart, CostBreakdown, DiscoveryOptions, FinishReason,
    Message, MessageRole, ModelCatalog, ModelInfo, ModelRef, PricingSource, ProviderCapabilities,
    ProviderId, ProviderRequest, ProviderResponse, ResponseFormat, RuntimeWarning, ToolChoice,
    ToolDefinition, Usage,
};
use crate::pricing::{PriceRule, PricingTable};

#[derive(Clone)]
struct MockAdapter {
    provider: ProviderId,
    capabilities: ProviderCapabilities,
    run_response: ProviderResponse,
    discovered_models: Vec<ModelInfo>,
}

impl MockAdapter {
    fn new(
        provider: ProviderId,
        capabilities: ProviderCapabilities,
        run_response: ProviderResponse,
        discovered_models: Vec<ModelInfo>,
    ) -> Self {
        Self {
            provider,
            capabilities,
            run_response,
            discovered_models,
        }
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
        _req: &ProviderRequest,
        _ctx: &AdapterContext,
    ) -> Result<ProviderResponse, ProviderError> {
        Ok(self.run_response.clone())
    }

    async fn discover_models(
        &self,
        _opts: &DiscoveryOptions,
        _ctx: &AdapterContext,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(self.discovered_models.clone())
    }
}

fn provider_capabilities(
    supports_tools: bool,
    supports_structured_output: bool,
    supports_remote_discovery: bool,
) -> ProviderCapabilities {
    ProviderCapabilities {
        supports_tools,
        supports_structured_output,
        supports_thinking: false,
        supports_remote_discovery,
    }
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

fn response(
    provider: ProviderId,
    model: &str,
    usage: Usage,
    cost: Option<CostBreakdown>,
    warnings: Vec<RuntimeWarning>,
) -> ProviderResponse {
    ProviderResponse {
        output: AssistantOutput {
            content: vec![ContentPart::Text {
                text: "ok".to_string(),
            }],
            structured_output: None,
        },
        usage,
        cost,
        provider,
        model: model.to_string(),
        raw_provider_response: None,
        finish_reason: FinishReason::Stop,
        warnings,
    }
}

fn request(
    provider_hint: Option<ProviderId>,
    model_id: &str,
    tools: Vec<ToolDefinition>,
    response_format: ResponseFormat,
) -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint,
            model_id: model_id.to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
        }],
        tools,
        tool_choice: ToolChoice::Auto,
        response_format,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn runtime_with_adapter(
    adapter: Arc<dyn ProviderAdapter>,
    pricing_table: Option<PricingTable>,
) -> ProviderRuntime {
    let builder = ProviderRuntime::builder().with_adapter(adapter);
    let builder = if let Some(table) = pricing_table {
        builder.with_pricing_table(table)
    } else {
        builder
    };
    builder.build()
}

#[tokio::test]
async fn test_runtime_run_routes_request() {
    let adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(true, true, false),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage::default(),
            None,
            Vec::new(),
        ),
        Vec::new(),
    ));

    let runtime = runtime_with_adapter(adapter, None);
    let req = request(
        Some(ProviderId::Openai),
        "gpt-5-mini",
        Vec::new(),
        ResponseFormat::Text,
    );

    let actual = runtime.run(req).await.expect("run should succeed");

    assert_eq!(actual.provider, ProviderId::Openai);
    assert_eq!(actual.model, "gpt-5-mini");
}

#[tokio::test]
async fn test_runtime_cost_attached_when_pricing_available() {
    let adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(true, true, false),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                reasoning_tokens: None,
                cached_input_tokens: None,
                total_tokens: None,
            },
            None,
            Vec::new(),
        ),
        Vec::new(),
    ));

    let pricing_table = PricingTable::new(vec![PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5-mini".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
        reasoning_cost_per_token: None,
    }]);

    let runtime = runtime_with_adapter(adapter, Some(pricing_table));
    let req = request(
        Some(ProviderId::Openai),
        "gpt-5-mini",
        Vec::new(),
        ResponseFormat::Text,
    );

    let actual = runtime.run(req).await.expect("run should succeed");

    let cost = actual.cost.expect("cost should be attached");
    assert_eq!(cost.currency, "USD");
    assert_eq!(cost.input_cost, 0.1);
    assert_eq!(cost.output_cost, 0.4);
    assert_eq!(cost.reasoning_cost, None);
    assert_eq!(cost.total_cost, 0.5);
    assert_eq!(cost.pricing_source, PricingSource::Configured);
    assert!(actual.warnings.is_empty());
}

#[tokio::test]
async fn test_runtime_discover_models_static_first() {
    let static_catalog = ModelCatalog {
        models: vec![model(
            ProviderId::Openai,
            "gpt-5-mini",
            Some("Static GPT"),
            Some(128_000),
            None,
        )],
    };

    let openai_adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(true, true, true),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage::default(),
            None,
            Vec::new(),
        ),
        vec![model(
            ProviderId::Openai,
            "gpt-5-mini",
            Some("Remote GPT"),
            Some(256_000),
            Some(16_000),
        )],
    ));

    let anthropic_adapter = Arc::new(MockAdapter::new(
        ProviderId::Anthropic,
        provider_capabilities(true, true, true),
        response(
            ProviderId::Anthropic,
            "claude-3-7-sonnet",
            Usage::default(),
            None,
            Vec::new(),
        ),
        vec![model(
            ProviderId::Anthropic,
            "claude-3-7-sonnet",
            Some("Claude"),
            Some(200_000),
            Some(8_000),
        )],
    ));

    let runtime = ProviderRuntime::builder()
        .with_model_catalog(static_catalog)
        .with_adapter(openai_adapter)
        .with_adapter(anthropic_adapter)
        .build();

    let actual = runtime
        .discover_models(DiscoveryOptions {
            remote: true,
            include_provider: Vec::new(),
            refresh_cache: true,
        })
        .await
        .expect("discover should succeed");

    assert_eq!(actual.models.len(), 2);
    assert_eq!(actual.models[0].provider, ProviderId::Openai);
    assert_eq!(actual.models[0].model_id, "gpt-5-mini");
    assert_eq!(actual.models[0].display_name.as_deref(), Some("Static GPT"));
    assert_eq!(actual.models[0].context_window, Some(128_000));
    assert_eq!(actual.models[0].max_output_tokens, Some(16_000));
    assert_eq!(actual.models[1].provider, ProviderId::Anthropic);
}

#[tokio::test]
async fn test_runtime_tools_capability_mismatch() {
    let adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(false, true, false),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage::default(),
            None,
            Vec::new(),
        ),
        Vec::new(),
    ));

    let runtime = runtime_with_adapter(adapter, None);
    let req = request(
        Some(ProviderId::Openai),
        "gpt-5-mini",
        vec![ToolDefinition {
            name: "lookup".to_string(),
            description: Some("tool".to_string()),
            parameters_schema: json!({"type":"object"}),
        }],
        ResponseFormat::Text,
    );

    let error = runtime
        .run(req)
        .await
        .expect_err("run should fail with capability mismatch");

    assert_eq!(
        error,
        crate::core::error::RuntimeError::CapabilityMismatch {
            provider: ProviderId::Openai,
            model: "gpt-5-mini".to_string(),
            capability: "tools".to_string(),
        }
    );
}

#[tokio::test]
async fn test_runtime_structured_output_capability_mismatch() {
    let adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(true, false, false),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage::default(),
            None,
            Vec::new(),
        ),
        Vec::new(),
    ));

    let runtime = runtime_with_adapter(adapter, None);
    let req = request(
        Some(ProviderId::Openai),
        "gpt-5-mini",
        Vec::new(),
        ResponseFormat::JsonObject,
    );

    let error = runtime
        .run(req)
        .await
        .expect_err("run should fail with capability mismatch");

    assert_eq!(
        error,
        crate::core::error::RuntimeError::CapabilityMismatch {
            provider: ProviderId::Openai,
            model: "gpt-5-mini".to_string(),
            capability: "structured_output".to_string(),
        }
    );
}

#[tokio::test]
async fn test_runtime_preserves_existing_provider_cost() {
    let provider_cost = CostBreakdown {
        currency: "USD".to_string(),
        input_cost: 1.0,
        output_cost: 2.0,
        reasoning_cost: None,
        total_cost: 3.0,
        pricing_source: PricingSource::ProviderReported,
    };

    let adapter = Arc::new(MockAdapter::new(
        ProviderId::Openai,
        provider_capabilities(true, true, false),
        response(
            ProviderId::Openai,
            "gpt-5-mini",
            Usage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                reasoning_tokens: None,
                cached_input_tokens: None,
                total_tokens: None,
            },
            Some(provider_cost.clone()),
            vec![RuntimeWarning {
                code: "provider.warning".to_string(),
                message: "from provider".to_string(),
            }],
        ),
        Vec::new(),
    ));

    let pricing_table = PricingTable::new(vec![PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5-mini".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
        reasoning_cost_per_token: None,
    }]);

    let runtime = runtime_with_adapter(adapter, Some(pricing_table));
    let req = request(
        Some(ProviderId::Openai),
        "gpt-5-mini",
        Vec::new(),
        ResponseFormat::Text,
    );

    let actual = runtime.run(req).await.expect("run should succeed");

    assert_eq!(actual.cost, Some(provider_cost));
    assert_eq!(actual.warnings.len(), 1);
    assert_eq!(actual.warnings[0].code, "provider.warning");
}
