use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use provider_runtime::core::error::*;
use provider_runtime::core::traits::*;
use provider_runtime::core::types::*;

struct CompileAdapter;

#[async_trait]
impl ProviderAdapter for CompileAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_tools: true,
            supports_structured_output: true,
            supports_thinking: false,
            supports_remote_discovery: false,
        }
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
            provider: ProviderId::Openai,
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
        Ok(Vec::new())
    }
}

struct CompileTokenProvider;

#[async_trait]
impl TokenProvider for CompileTokenProvider {
    async fn get_token(&self, _provider: ProviderId) -> Result<String, RuntimeError> {
        Ok("token".to_string())
    }
}

#[tokio::test]
async fn test_core_exports_compile() {
    let request = ProviderRequest {
        model: ModelRef {
            provider_hint: Some(ProviderId::Openai),
            model_id: "gpt-5-mini".to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
        }],
        tools: vec![ToolDefinition {
            name: "lookup".to_string(),
            description: Some("tool".to_string()),
            parameters_schema: serde_json::json!({ "type": "object" }),
        }],
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    };
    let ctx = AdapterContext::default();
    let opts = DiscoveryOptions {
        remote: false,
        include_provider: vec![ProviderId::Openai],
        refresh_cache: false,
    };

    let _config_error = ConfigError::MissingDefaultProvider;
    let _routing_error = RoutingError::ModelNotFound {
        model: "gpt-5-mini".to_string(),
    };
    let _provider_error = ProviderError::Transport {
        provider: ProviderId::Openai,
        request_id: Some("req_123".to_string()),
        message: "timeout".to_string(),
    };
    let _runtime_error =
        RuntimeError::credential_missing(ProviderId::Openai, vec!["OPENAI_API_KEY".to_string()]);

    let adapter: Arc<dyn ProviderAdapter> = Arc::new(CompileAdapter);
    let token_provider: Arc<dyn TokenProvider> = Arc::new(CompileTokenProvider);

    let run_result: Result<ProviderResponse, ProviderError> = adapter.run(&request, &ctx).await;
    assert!(run_result.is_ok());

    let discover_result: Result<Vec<ModelInfo>, ProviderError> =
        adapter.discover_models(&opts, &ctx).await;
    assert!(discover_result.is_ok());

    let token_result: Result<String, RuntimeError> =
        token_provider.get_token(ProviderId::Openai).await;
    assert!(token_result.is_ok());
}
