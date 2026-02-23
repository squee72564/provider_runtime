use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub model: ModelRef,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default)]
    pub tool_choice: ToolChoice,
    #[serde(default)]
    pub response_format: ResponseFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    pub output: AssistantOutput,
    pub usage: Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostBreakdown>,
    pub provider: ProviderId,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_provider_response: Option<serde_json::Value>,
    pub finish_reason: FinishReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_hint: Option<ProviderId>,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        provider: Option<ProviderId>,
    },
    ToolCall {
        tool_call: ToolCall,
    },
    ToolResult {
        tool_result: ToolResult,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Specific,
}

impl Default for ToolChoice {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
    },
}

impl Default for ResponseFormat {
    fn default() -> Self {
        Self::Text
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOutput {
    pub content: Vec<ContentPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl Usage {
    pub fn derived_total_tokens(&self) -> u64 {
        self.total_tokens
            .unwrap_or(self.input_tokens.unwrap_or(0) + self.output_tokens.unwrap_or(0))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostBreakdown {
    pub currency: String,
    pub input_cost: f64,
    pub output_cost: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_cost: Option<f64>,
    pub total_cost: f64,
    pub pricing_source: PricingSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PricingSource {
    Configured,
    ProviderReported,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInfo {
    pub provider: ProviderId,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    pub supports_tools: bool,
    pub supports_structured_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalog {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryOptions {
    pub remote: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_provider: Vec<ProviderId>,
    pub refresh_cache: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderId {
    Openai,
    Anthropic,
    Openrouter,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    pub supports_tools: bool,
    pub supports_structured_output: bool,
    pub supports_thinking: bool,
    pub supports_remote_discovery: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AdapterContext {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_provider_request_serde_roundtrip() {
        let mut metadata = BTreeMap::new();
        metadata.insert("trace_id".to_string(), "abc-123".to_string());

        let req = ProviderRequest {
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
                description: None,
                parameters_schema: json!({"type": "object"}),
            }],
            tool_choice: ToolChoice::Auto,
            response_format: ResponseFormat::JsonSchema {
                name: "answer".to_string(),
                schema: json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}),
            },
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            stop: Vec::new(),
            metadata,
        };

        let value = serde_json::to_value(&req).expect("request should serialize");

        assert!(value.get("temperature").is_none());
        assert!(value.get("top_p").is_none());
        assert!(value.get("max_output_tokens").is_none());
        assert!(value.get("stop").is_none());

        let response_format = value
            .get("response_format")
            .expect("response_format should exist");
        assert_eq!(response_format.get("type"), Some(&json!("json_schema")));

        let roundtrip: ProviderRequest =
            serde_json::from_value(value).expect("request should deserialize");
        assert_eq!(roundtrip, req);
    }

    #[test]
    fn test_usage_total_tokens_derivation() {
        let explicit = Usage {
            input_tokens: Some(2),
            output_tokens: Some(3),
            reasoning_tokens: Some(5),
            cached_input_tokens: Some(7),
            total_tokens: Some(99),
        };
        assert_eq!(explicit.derived_total_tokens(), 99);

        let derived = Usage {
            input_tokens: Some(2),
            output_tokens: Some(3),
            reasoning_tokens: Some(100),
            cached_input_tokens: Some(100),
            total_tokens: None,
        };
        assert_eq!(derived.derived_total_tokens(), 5);

        let zero_based = Usage {
            input_tokens: None,
            output_tokens: Some(4),
            reasoning_tokens: None,
            cached_input_tokens: None,
            total_tokens: None,
        };
        assert_eq!(zero_based.derived_total_tokens(), 4);
    }

    #[test]
    fn test_content_part_invariants() {
        let part = ContentPart::ToolResult {
            tool_result: ToolResult {
                tool_call_id: "call_1".to_string(),
                content: vec![ContentPart::Text {
                    text: "done".to_string(),
                }],
            },
        };

        let value = serde_json::to_value(&part).expect("content part should serialize");
        assert_eq!(value.get("type"), Some(&json!("tool_result")));
        assert_eq!(
            value.pointer("/tool_result/tool_call_id"),
            Some(&json!("call_1"))
        );

        let invalid_message = json!({
            "role": { "type": "user" },
            "content": [],
            "unexpected": true
        });
        let err = serde_json::from_value::<Message>(invalid_message)
            .expect_err("unknown fields should fail for Message");
        assert!(err.to_string().contains("unknown field"));
    }
}
