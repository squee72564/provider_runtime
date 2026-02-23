use std::collections::BTreeMap;

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
