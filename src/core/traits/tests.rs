use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::*;
use crate::core::error::{ProviderError, RuntimeError};
use crate::core::types::{
    AdapterContext, AssistantOutput, ContentPart, DiscoveryOptions, FinishReason, Message,
    MessageRole, ModelInfo, ModelRef, ProviderCapabilities, ProviderId, ProviderRequest,
    ProviderResponse, ResponseFormat, ToolChoice, Usage,
};

#[derive(Clone)]
struct MockAdapter {
    provider: ProviderId,
    capabilities: ProviderCapabilities,
    run_calls: Arc<Mutex<u32>>,
    discover_calls: Arc<Mutex<u32>>,
}

impl MockAdapter {
    fn new(provider: ProviderId, capabilities: ProviderCapabilities) -> Self {
        Self {
            provider,
            capabilities,
            run_calls: Arc::new(Mutex::new(0)),
            discover_calls: Arc::new(Mutex::new(0)),
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
        req: &ProviderRequest,
        _ctx: &AdapterContext,
    ) -> Result<ProviderResponse, ProviderError> {
        *self
            .run_calls
            .lock()
            .expect("run lock should not be poisoned") += 1;

        Ok(ProviderResponse {
            output: AssistantOutput {
                content: vec![ContentPart::Text {
                    text: "ok".to_string(),
                }],
                structured_output: None,
            },
            usage: Usage {
                input_tokens: Some(1),
                output_tokens: Some(1),
                cached_input_tokens: None,
                total_tokens: None,
            },
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
            .expect("discover lock should not be poisoned") += 1;

        Ok(vec![ModelInfo {
            provider: self.provider.clone(),
            model_id: "mock-model".to_string(),
            display_name: Some("Mock Model".to_string()),
            context_window: Some(8_192),
            max_output_tokens: Some(1_024),
            supports_tools: self.capabilities.supports_tools,
            supports_structured_output: self.capabilities.supports_structured_output,
        }])
    }
}

struct MockTokenProvider {
    fail_provider: Option<ProviderId>,
    failure: RuntimeError,
    calls: Arc<Mutex<Vec<ProviderId>>>,
}

impl MockTokenProvider {
    fn new(fail_provider: Option<ProviderId>, failure: RuntimeError) -> Self {
        Self {
            fail_provider,
            failure,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl TokenProvider for MockTokenProvider {
    async fn get_token(&self, provider: ProviderId) -> Result<String, RuntimeError> {
        self.calls
            .lock()
            .expect("calls lock should not be poisoned")
            .push(provider.clone());

        if self.fail_provider == Some(provider.clone()) {
            return Err(self.failure.clone());
        }

        Ok(format!("token_for_{provider:?}"))
    }
}

fn sample_request() -> ProviderRequest {
    ProviderRequest {
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
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

#[tokio::test]
async fn test_provider_adapter_object_safety() {
    let capabilities = ProviderCapabilities {
        supports_tools: true,
        supports_structured_output: true,
        supports_thinking: false,
        supports_remote_discovery: true,
    };
    let adapter: Box<dyn ProviderAdapter> =
        Box::new(MockAdapter::new(ProviderId::Openai, capabilities.clone()));

    assert_eq!(adapter.id(), ProviderId::Openai);
    assert_eq!(adapter.capabilities(), capabilities);

    let borrowed: &dyn ProviderAdapter = adapter.as_ref();
    assert_eq!(borrowed.id(), ProviderId::Openai);
    assert_eq!(borrowed.capabilities(), capabilities);

    let req = sample_request();
    let ctx = AdapterContext::default();
    let response = borrowed
        .run(&req, &ctx)
        .await
        .expect("mock run should succeed");
    assert_eq!(response.provider, ProviderId::Openai);
    assert_eq!(response.model, "gpt-5-mini");

    let models = adapter
        .discover_models(
            &DiscoveryOptions {
                remote: false,
                include_provider: Vec::new(),
                refresh_cache: false,
            },
            &ctx,
        )
        .await
        .expect("mock discovery should succeed");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].provider, ProviderId::Openai);
}

#[test]
fn test_provider_capabilities_contract() {
    let expected = ProviderCapabilities {
        supports_tools: false,
        supports_structured_output: true,
        supports_thinking: true,
        supports_remote_discovery: false,
    };
    let adapter = MockAdapter::new(ProviderId::Anthropic, expected.clone());

    assert_eq!(adapter.id(), ProviderId::Anthropic);
    assert_eq!(adapter.capabilities(), expected);
    assert_eq!(adapter.capabilities(), expected);
}

#[tokio::test]
async fn test_token_provider_provider_scoped_contract() {
    let failure = RuntimeError::TransportError {
        provider: Some(ProviderId::Anthropic),
        model: None,
        request_id: Some("req_token_1".to_string()),
        message: "token endpoint unavailable".to_string(),
    };
    let token_provider = MockTokenProvider::new(Some(ProviderId::Anthropic), failure.clone());

    let ok = token_provider
        .get_token(ProviderId::Openai)
        .await
        .expect("openai token should succeed");
    assert_eq!(ok, "token_for_Openai");

    let err = token_provider
        .get_token(ProviderId::Anthropic)
        .await
        .expect_err("anthropic token should fail");
    assert_eq!(err, failure);

    let calls = token_provider
        .calls
        .lock()
        .expect("calls lock should not be poisoned")
        .clone();
    assert_eq!(calls, vec![ProviderId::Openai, ProviderId::Anthropic]);
}
