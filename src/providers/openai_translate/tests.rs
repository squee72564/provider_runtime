use std::collections::BTreeMap;

use serde_json::json;

use super::{
    OpenAiDecodeEnvelope, decode_openai_models_list, decode_openai_response, encode_openai_request,
    format_openai_error_message, parse_openai_error_envelope,
};
use crate::core::error::ProviderError;
use crate::core::types::{
    ContentPart, FinishReason, Message, MessageRole, ModelRef, ProviderCapabilities, ProviderId,
    ProviderRequest, ResponseFormat, ToolChoice, ToolDefinition, ToolResultContent,
};

fn base_request() -> ProviderRequest {
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
fn test_encode_openai_translator_category_contract() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::System,
            content: vec![ContentPart::Text {
                text: "You must return JSON.".to_string(),
            }],
        },
        Message {
            role: MessageRole::User,
            content: vec![
                ContentPart::Text {
                    text: "Respond in JSON with weather details".to_string(),
                },
                ContentPart::Thinking {
                    text: "internal".to_string(),
                    provider: Some(ProviderId::Openai),
                },
            ],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: crate::core::types::ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({ "city": "SF" }),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: crate::core::types::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    content: ToolResultContent::Text {
                        text: "{\"temp\":55}".to_string(),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];
    req.tools = vec![ToolDefinition {
        name: "lookup_weather".to_string(),
        description: Some("Lookup weather by city".to_string()),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": []
        }),
    }];
    req.tool_choice = ToolChoice::Specific {
        name: "lookup_weather".to_string(),
    };
    req.response_format = ResponseFormat::JsonSchema {
        name: "weather_response".to_string(),
        schema: json!({
            "type": "object",
            "properties": {"temp": {"type": "number"}},
            "required": ["temp"],
            "additionalProperties": false
        }),
    };
    req.temperature = Some(0.6);
    req.top_p = Some(0.9);
    req.max_output_tokens = Some(128);
    req.metadata
        .insert("trace_id".to_string(), "abc-123".to_string());

    let encoded = encode_openai_request(&req).expect("encode should succeed");

    assert_eq!(encoded.body.pointer("/model"), Some(&json!("gpt-5-mini")));
    assert_eq!(encoded.body.pointer("/store"), Some(&json!(false)));
    assert_eq!(
        encoded.body.pointer("/text/format/type"),
        Some(&json!("json_schema"))
    );
    assert_eq!(
        encoded.body.pointer("/tool_choice/type"),
        Some(&json!("function"))
    );
    assert_eq!(
        encoded.body.pointer("/tool_choice/name"),
        Some(&json!("lookup_weather"))
    );
    assert_eq!(
        encoded.body.pointer("/max_output_tokens"),
        Some(&json!(128))
    );

    let input = encoded
        .body
        .get("input")
        .and_then(|value| value.as_array())
        .expect("input should be array");
    assert_eq!(input.len(), 4);
    assert_eq!(input[0].pointer("/role"), Some(&json!("system")));
    assert_eq!(input[1].pointer("/role"), Some(&json!("user")));
    assert_eq!(input[2].pointer("/type"), Some(&json!("function_call")));
    assert_eq!(
        input[3].pointer("/type"),
        Some(&json!("function_call_output"))
    );

    let warning_codes = encoded
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect::<Vec<_>>();
    assert!(warning_codes.contains(&"dropped_thinking_on_encode"));
    assert!(warning_codes.contains(&"both_temperature_and_top_p_set"));
    assert!(warning_codes.contains(&"tool_schema_not_strict_compatible_strict_disabled"));
}

#[test]
fn test_decode_openai_translator_category_contract() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "completed",
            "model": "gpt-5-mini",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "{\"ok\":true}" }
                    ]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup_weather",
                    "arguments": "{\"city\":\"SF\"}"
                },
                {
                    "type": "reasoning",
                    "text": "thinking summary"
                }
            ],
            "usage": {
                "input_tokens": 11,
                "output_tokens": 7,
                "total_tokens": 18,
                "input_tokens_details": { "cached_tokens": 2 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_openai_response(&payload).expect("decode should succeed");

    assert_eq!(decoded.provider, ProviderId::Openai);
    assert_eq!(decoded.model, "gpt-5-mini");
    assert_eq!(decoded.finish_reason, FinishReason::ToolCalls);
    assert_eq!(decoded.usage.input_tokens, Some(11));
    assert_eq!(decoded.usage.output_tokens, Some(7));
    assert_eq!(decoded.usage.total_tokens, Some(18));
    assert_eq!(decoded.usage.cached_input_tokens, Some(2));
    assert_eq!(decoded.usage.reasoning_tokens, Some(3));
    assert_eq!(
        decoded.output.structured_output,
        Some(json!({ "ok": true }))
    );

    assert_eq!(decoded.output.content.len(), 3);
    assert!(matches!(
        &decoded.output.content[0],
        ContentPart::Text { text } if text == "{\"ok\":true}"
    ));
    assert!(matches!(
        &decoded.output.content[1],
        ContentPart::ToolCall { tool_call } if tool_call.id == "call_1" && tool_call.name == "lookup_weather"
    ));
    assert!(matches!(
        &decoded.output.content[2],
        ContentPart::Thinking { provider, .. } if provider == &Some(ProviderId::Openai)
    ));
}

#[test]
fn test_openai_translator_determinism_contract() {
    let req = base_request();
    let first_encode = encode_openai_request(&req).expect("encode should succeed");
    let second_encode = encode_openai_request(&req).expect("encode should succeed");
    assert_eq!(first_encode, second_encode);

    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "completed",
            "model": "gpt-5-mini",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "done" }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2
            }
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let first_decode = decode_openai_response(&payload).expect("decode should succeed");
    let second_decode = decode_openai_response(&payload).expect("decode should succeed");
    assert_eq!(first_decode, second_decode);
}

#[test]
fn test_encode_stop_is_unsupported() {
    let mut req = base_request();
    req.stop.push("STOP".to_string());

    let err = encode_openai_request(&req).expect_err("stop should be unsupported");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("stop sequences are unsupported"));
}

#[test]
fn test_decode_incomplete_content_filter_maps_finish_reason() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "incomplete",
            "model": "gpt-5-mini",
            "incomplete_details": { "reason": "content_filter" },
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "filtered" }
                    ]
                }
            ],
            "usage": null
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded = decode_openai_response(&payload).expect("decode should succeed");
    assert_eq!(decoded.finish_reason, FinishReason::ContentFilter);
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "openai_incomplete_content_filter")
    );
}

#[test]
fn test_decode_structured_output_parse_failure_warns() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "completed",
            "model": "gpt-5-mini",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "not-json" }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2
            }
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_openai_response(&payload).expect("decode should succeed");
    assert_eq!(decoded.output.structured_output, None);
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "structured_output_parse_failed")
    );
}

#[test]
fn test_decode_cancelled_status_is_protocol_error() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "cancelled",
            "model": "gpt-5-mini",
            "output": [],
            "usage": null
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let err = decode_openai_response(&payload).expect_err("cancelled must be protocol error");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("status is cancelled"));
}

#[test]
fn test_decode_unknown_output_item_type_is_protocol_error() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "completed",
            "model": "gpt-5-mini",
            "output": [
                { "type": "web_search_call" }
            ],
            "usage": {
                "input_tokens": 1,
                "output_tokens": 1,
                "total_tokens": 2
            }
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let err = decode_openai_response(&payload).expect_err("unknown types must fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("unsupported output item type"));
}

#[test]
fn test_decode_tool_call_invalid_json_warns_and_preserves_raw_string() {
    let payload = OpenAiDecodeEnvelope {
        body: json!({
            "status": "completed",
            "model": "gpt-5-mini",
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_bad",
                    "name": "lookup_weather",
                    "arguments": "{not-valid-json"
                }
            ],
            "usage": {
                "input_tokens": 2,
                "output_tokens": 3,
                "total_tokens": 5
            }
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded = decode_openai_response(&payload).expect("decode should succeed");
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "tool_arguments_invalid_json")
    );
    assert!(matches!(
        &decoded.output.content[0],
        ContentPart::ToolCall { tool_call }
            if tool_call.id == "call_bad"
                && tool_call.name == "lookup_weather"
                && tool_call.arguments_json == json!("{not-valid-json")
    ));
}

#[test]
fn test_encode_json_object_requires_json_keyword() {
    let mut req = base_request();
    req.response_format = ResponseFormat::JsonObject;

    let err = encode_openai_request(&req).expect_err("JSON keyword check should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("requires the string 'JSON'"));
}

#[test]
fn test_encode_metadata_bounds_validation() {
    let mut req = base_request();
    for index in 0..17 {
        req.metadata
            .insert(format!("k{index}"), format!("value-{index}"));
    }

    let err = encode_openai_request(&req).expect_err("metadata size should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("at most 16 entries"));
}

#[test]
fn test_encode_tool_result_role_validation() {
    let mut req = base_request();
    req.messages = vec![Message {
        role: MessageRole::User,
        content: vec![ContentPart::ToolResult {
            tool_result: crate::core::types::ToolResult {
                tool_call_id: "call_1".to_string(),
                content: ToolResultContent::Text {
                    text: "result".to_string(),
                },
                raw_provider_content: None,
            },
        }],
    }];

    let err = encode_openai_request(&req).expect_err("tool_result role mismatch should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("tool_result content is only valid for tool role")
    );
}

#[test]
fn test_encode_tool_result_json_coerces_to_string() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: crate::core::types::ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: crate::core::types::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    content: ToolResultContent::Json {
                        value: json!({"b": 2, "a": 1}),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let encoded = encode_openai_request(&req).expect("encode should succeed");
    assert_eq!(
        encoded.body.pointer("/input/1/output"),
        Some(&json!("{\"a\":1,\"b\":2}"))
    );
    assert!(
        encoded
            .warnings
            .iter()
            .any(|warning| warning.code == "tool_result_coerced")
    );
}

#[test]
fn test_encode_tool_result_uses_raw_provider_content_when_string() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: crate::core::types::ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: crate::core::types::ToolResult {
                    tool_call_id: "call_1".to_string(),
                    content: ToolResultContent::Json {
                        value: json!({"a": 1}),
                    },
                    raw_provider_content: Some(json!("raw-output")),
                },
            }],
        },
    ];

    let encoded = encode_openai_request(&req).expect("encode should succeed");
    assert_eq!(
        encoded.body.pointer("/input/1/output"),
        Some(&json!("raw-output"))
    );
    assert!(
        encoded
            .warnings
            .iter()
            .all(|warning| warning.code != "tool_result_coerced")
    );
}

#[test]
fn test_encode_tool_choice_specific_requires_declared_tool() {
    let mut req = base_request();
    req.tool_choice = ToolChoice::Specific {
        name: "missing_tool".to_string(),
    };

    let err = encode_openai_request(&req).expect_err("unknown specific tool must fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("references unknown tool"));
}

#[test]
fn test_parse_openai_error_envelope_and_format() {
    let envelope = parse_openai_error_envelope(
        r#"{
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        }"#,
    )
    .expect("should parse");

    assert_eq!(envelope.message, "Invalid API key");
    assert_eq!(envelope.code.as_deref(), Some("invalid_api_key"));
    assert_eq!(
        envelope.error_type.as_deref(),
        Some("invalid_request_error")
    );
    assert_eq!(envelope.param, None);

    let message = format_openai_error_message(&envelope);
    assert!(message.contains("openai error: Invalid API key"));
    assert!(message.contains("code=invalid_api_key"));
    assert!(message.contains("type=invalid_request_error"));
}

#[test]
fn test_decode_openai_models_list_success() {
    let capabilities = ProviderCapabilities {
        supports_tools: true,
        supports_structured_output: true,
        supports_thinking: false,
        supports_remote_discovery: true,
    };

    let models = decode_openai_models_list(
        &json!({
            "object": "list",
            "data": [
                {"id":"gpt-5-mini","object":"model"},
                {"id":"gpt-4.1","object":"model"},
                {"id":"gpt-5-mini","object":"model"}
            ]
        }),
        &capabilities,
    )
    .expect("decode should succeed");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model_id, "gpt-5-mini");
    assert_eq!(models[1].model_id, "gpt-4.1");
    assert!(models.iter().all(|model| model.supports_tools));
    assert!(models.iter().all(|model| model.supports_structured_output));
}

#[test]
fn test_decode_openai_models_list_rejects_missing_data_array() {
    let capabilities = ProviderCapabilities {
        supports_tools: true,
        supports_structured_output: true,
        supports_thinking: false,
        supports_remote_discovery: true,
    };

    let err = decode_openai_models_list(&json!({"object":"list"}), &capabilities)
        .expect_err("missing data must fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("missing data array"));
}

#[test]
fn test_encode_openai_request_always_sets_store_false() {
    let req = base_request();
    let encoded = encode_openai_request(&req).expect("encode should succeed");
    assert_eq!(encoded.body.pointer("/store"), Some(&json!(false)));
}
