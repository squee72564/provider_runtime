use crate::core::error::ProviderError;
use crate::core::types::{ProviderRequest, ProviderResponse};

/// Internal provider-layer translation contract.
///
/// `ProviderAdapter` remains the runtime-facing public extension point for
/// orchestration (auth, transport, capability declaration, and discovery).
/// This contract is crate-private and used by provider modules to normalize
/// canonical runtime types to provider protocol payloads and back.
#[allow(dead_code)]
pub(crate) trait ProviderTranslator {
    /// Provider protocol payload used for outbound request encoding.
    type RequestPayload;

    /// Provider protocol payload used for inbound response decoding.
    type ResponsePayload;

    /// Encodes canonical request semantics into a provider protocol payload.
    fn encode_request(&self, req: &ProviderRequest) -> Result<Self::RequestPayload, ProviderError>;

    /// Decodes a provider protocol payload into canonical runtime response semantics.
    fn decode_response(
        &self,
        payload: &Self::ResponsePayload,
    ) -> Result<ProviderResponse, ProviderError>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::ProviderTranslator;
    use crate::core::error::ProviderError;
    use crate::core::types::{
        AssistantOutput, ContentPart, FinishReason, Message, MessageRole, ModelRef, ProviderId,
        ProviderRequest, ProviderResponse, ResponseFormat, ToolChoice, Usage,
    };

    struct MockTranslator;

    impl ProviderTranslator for MockTranslator {
        type RequestPayload = Value;
        type ResponsePayload = Value;

        fn encode_request(
            &self,
            req: &ProviderRequest,
        ) -> Result<Self::RequestPayload, ProviderError> {
            Ok(json!({
                "model": req.model.model_id,
                "message_count": req.messages.len(),
            }))
        }

        fn decode_response(
            &self,
            payload: &Self::ResponsePayload,
        ) -> Result<ProviderResponse, ProviderError> {
            let text = payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();

            Ok(ProviderResponse {
                output: AssistantOutput {
                    content: vec![ContentPart::Text { text }],
                    structured_output: None,
                },
                usage: Usage::default(),
                cost: None,
                provider: ProviderId::Openai,
                model: "mock-model".to_string(),
                raw_provider_response: None,
                finish_reason: FinishReason::Stop,
                warnings: Vec::new(),
            })
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

    #[test]
    fn test_provider_translator_trait_shape_encode_decode() {
        let translator = MockTranslator;

        let encoded = translator
            .encode_request(&sample_request())
            .expect("encode should succeed");
        assert_eq!(encoded.get("model"), Some(&json!("gpt-5-mini")));
        assert_eq!(encoded.get("message_count"), Some(&json!(1)));

        let decoded = translator
            .decode_response(&json!({ "text": "done" }))
            .expect("decode should succeed");
        assert_eq!(decoded.provider, ProviderId::Openai);
        assert_eq!(decoded.model, "mock-model");
        assert_eq!(
            decoded.output.content,
            vec![ContentPart::Text {
                text: "done".to_string(),
            }]
        );
    }
}
