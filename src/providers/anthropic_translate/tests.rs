use std::collections::BTreeMap;

use serde_json::json;

use super::{
    AnthropicDecodeEnvelope, decode_anthropic_models_list, decode_anthropic_response,
    encode_anthropic_request, format_anthropic_error_message, parse_anthropic_error_envelope,
};
use crate::core::error::ProviderError;
use crate::core::types::{
    ContentPart, FinishReason, Message, MessageRole, ModelRef, ProviderCapabilities, ProviderId,
    ProviderRequest, ResponseFormat, ToolCall, ToolChoice, ToolDefinition, ToolResult,
    ToolResultContent,
};

fn base_request() -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(ProviderId::Anthropic),
            model_id: "claude-sonnet-4-5".to_string(),
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
fn test_encode_anthropic_translator_category_contract() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::System,
            content: vec![ContentPart::Text {
                text: "You are a concise assistant.".to_string(),
            }],
        },
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Return weather as JSON".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({ "city": "SF" }),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "tool_1".to_string(),
                    content: ToolResultContent::Text {
                        text: "{\"temp\":55}".to_string(),
                    },
                    raw_provider_content: None,
                },
            }],
        },
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Now summarize".to_string(),
            }],
        },
    ];
    req.tools = vec![ToolDefinition {
        name: "lookup_weather".to_string(),
        description: Some("Lookup weather".to_string()),
        parameters_schema: json!({
            "type": "object",
            "properties": {"city": {"type":"string"}},
            "required": ["city"]
        }),
    }];
    req.tool_choice = ToolChoice::Specific {
        name: "lookup_weather".to_string(),
    };
    req.response_format = ResponseFormat::JsonSchema {
        name: "weather".to_string(),
        schema: json!({
            "type": "object",
            "properties": {"temp": {"type":"number"}},
            "required": ["temp"]
        }),
    };
    req.temperature = Some(0.2);
    req.top_p = Some(0.8);
    req.metadata
        .insert("user_id".to_string(), "abc123".to_string());
    req.metadata
        .insert("trace_id".to_string(), "trace-1".to_string());
    req.stop.push("DONE".to_string());

    let encoded = encode_anthropic_request(&req).expect("encode should succeed");

    assert_eq!(
        encoded.body.pointer("/model"),
        Some(&json!("claude-sonnet-4-5"))
    );
    assert_eq!(encoded.body.pointer("/max_tokens"), Some(&json!(1024)));
    assert_eq!(
        encoded.body.pointer("/tool_choice/type"),
        Some(&json!("tool"))
    );
    assert_eq!(
        encoded.body.pointer("/tool_choice/name"),
        Some(&json!("lookup_weather"))
    );
    assert_eq!(
        encoded
            .body
            .pointer("/tool_choice/disable_parallel_tool_use"),
        Some(&json!(true))
    );
    assert_eq!(
        encoded.body.pointer("/output_config/format/type"),
        Some(&json!("json_schema"))
    );
    assert_eq!(
        encoded.body.pointer("/metadata/user_id"),
        Some(&json!("abc123"))
    );

    let messages = encoded
        .body
        .get("messages")
        .and_then(|value| value.as_array())
        .expect("messages should be an array");
    let tool_result_user_message = messages
        .iter()
        .find_map(|message| {
            let role = message.get("role")?.as_str()?;
            if role != "user" {
                return None;
            }
            let content = message.get("content")?.as_array()?;
            let first_type = content
                .first()?
                .get("type")
                .and_then(|value| value.as_str())?;
            if first_type == "tool_result" {
                Some(content)
            } else {
                None
            }
        })
        .expect("expected at least one user message starting with tool_result");
    assert_eq!(
        tool_result_user_message[0].pointer("/type"),
        Some(&json!("tool_result"))
    );

    let warning_codes = encoded
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect::<Vec<_>>();
    assert!(warning_codes.contains(&"both_temperature_and_top_p_set"));
    assert!(warning_codes.contains(&"default_max_tokens_applied"));
    assert!(warning_codes.contains(&"dropped_unsupported_metadata_keys"));
}

#[test]
fn test_encode_minimal_text_request() {
    let encoded = encode_anthropic_request(&base_request()).expect("encode should succeed");

    assert_eq!(
        encoded.body.pointer("/model"),
        Some(&json!("claude-sonnet-4-5"))
    );
    assert_eq!(encoded.body.pointer("/max_tokens"), Some(&json!(1024)));
    assert_eq!(
        encoded.body.pointer("/messages/0/role"),
        Some(&json!("user"))
    );
    assert_eq!(
        encoded.body.pointer("/messages/0/content/0/type"),
        Some(&json!("text"))
    );
    assert_eq!(
        encoded.body.pointer("/tool_choice/type"),
        Some(&json!("auto"))
    );
}

#[test]
fn test_encode_tool_choice_mode_matrix() {
    let mut req = base_request();
    req.tools = vec![ToolDefinition {
        name: "lookup".to_string(),
        description: Some("Lookup".to_string()),
        parameters_schema: json!({"type":"object"}),
    }];

    req.tool_choice = ToolChoice::None;
    let none_choice = encode_anthropic_request(&req).expect("none should encode");
    assert_eq!(
        none_choice.body.pointer("/tool_choice/type"),
        Some(&json!("none"))
    );

    req.tool_choice = ToolChoice::Auto;
    let auto_choice = encode_anthropic_request(&req).expect("auto should encode");
    assert_eq!(
        auto_choice.body.pointer("/tool_choice/type"),
        Some(&json!("auto"))
    );

    req.tool_choice = ToolChoice::Required;
    let required_choice = encode_anthropic_request(&req).expect("required should encode");
    assert_eq!(
        required_choice.body.pointer("/tool_choice/type"),
        Some(&json!("any"))
    );

    req.tool_choice = ToolChoice::Specific {
        name: "lookup".to_string(),
    };
    let specific_choice = encode_anthropic_request(&req).expect("specific should encode");
    assert_eq!(
        specific_choice.body.pointer("/tool_choice/type"),
        Some(&json!("tool"))
    );
    assert_eq!(
        specific_choice
            .body
            .pointer("/tool_choice/disable_parallel_tool_use"),
        Some(&json!(true))
    );
}

#[test]
fn test_encode_tool_choice_requires_tools_for_required_and_specific() {
    let mut req = base_request();
    req.tools = Vec::new();

    req.tool_choice = ToolChoice::Required;
    let required_err = encode_anthropic_request(&req).expect_err("required should fail");
    assert!(
        required_err
            .to_string()
            .contains("requires at least one tool definition")
    );

    req.tool_choice = ToolChoice::Specific {
        name: "lookup".to_string(),
    };
    let specific_err = encode_anthropic_request(&req).expect_err("specific should fail");
    assert!(
        specific_err
            .to_string()
            .contains("requires at least one tool definition")
    );
}

#[test]
fn test_encode_response_format_matrix() {
    let mut req = base_request();
    req.response_format = ResponseFormat::JsonObject;
    let json_object = encode_anthropic_request(&req).expect("json object encode");
    assert_eq!(
        json_object.body.pointer("/output_config/format/type"),
        Some(&json!("json_schema"))
    );
    assert_eq!(
        json_object
            .body
            .pointer("/output_config/format/schema/type"),
        Some(&json!("object"))
    );

    req.response_format = ResponseFormat::JsonSchema {
        name: "shape".to_string(),
        schema: json!({"type":"object","properties":{"value":{"type":"number"}}}),
    };
    let json_schema = encode_anthropic_request(&req).expect("json schema encode");
    assert_eq!(
        json_schema.body.pointer("/output_config/format/type"),
        Some(&json!("json_schema"))
    );
    assert_eq!(
        json_schema
            .body
            .pointer("/output_config/format/schema/properties/value/type"),
        Some(&json!("number"))
    );
}

#[test]
fn test_encode_json_prefill_is_rejected() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "output json".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Text {
                text: "{".to_string(),
            }],
        },
    ];
    req.response_format = ResponseFormat::JsonObject;

    let err = encode_anthropic_request(&req).expect_err("prefill should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("incompatible with assistant-prefill final messages")
    );
}

#[test]
fn test_encode_non_prefix_system_is_rejected() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
        },
        Message {
            role: MessageRole::System,
            content: vec![ContentPart::Text {
                text: "late system".to_string(),
            }],
        },
    ];

    let err = encode_anthropic_request(&req).expect_err("late system should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}

#[test]
fn test_encode_max_output_tokens_zero_is_rejected() {
    let mut req = base_request();
    req.max_output_tokens = Some(0);

    let err = encode_anthropic_request(&req).expect_err("max_output_tokens=0 should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("max_output_tokens must be at least 1")
    );
}

#[test]
fn test_encode_tool_choice_specific_requires_declared_tool() {
    let mut req = base_request();
    req.tools = vec![ToolDefinition {
        name: "lookup_weather".to_string(),
        description: None,
        parameters_schema: json!({"type":"object"}),
    }];
    req.tool_choice = ToolChoice::Specific {
        name: "missing_tool".to_string(),
    };

    let err = encode_anthropic_request(&req).expect_err("specific tool should be validated");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("tool_choice specific references unknown tool")
    );
}

#[test]
fn test_encode_tool_call_arguments_must_be_object() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "call tool".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!(["not-object"]),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "tool_1".to_string(),
                    content: ToolResultContent::Text {
                        text: "{\"temp\":55}".to_string(),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let err = encode_anthropic_request(&req).expect_err("non-object tool args should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("arguments_json must be a JSON object")
    );
}

#[test]
fn test_encode_tool_result_without_matching_tool_call_is_rejected() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "hello".to_string(),
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "missing_call".to_string(),
                    content: ToolResultContent::Text {
                        text: "tool output".to_string(),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let err = encode_anthropic_request(&req).expect_err("missing tool call should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("tool_result references unknown tool_call_id")
    );
}

#[test]
fn test_encode_assistant_tool_use_requires_following_tool_result() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "call the tool".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "where are we now?".to_string(),
            }],
        },
    ];

    let err = encode_anthropic_request(&req).expect_err("missing tool_result should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("requires tool_result blocks at the start of the next user message")
    );
}

#[test]
fn test_encode_tool_result_content_non_text_is_rejected() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "call the tool".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "tool_1".to_string(),
                    content: ToolResultContent::Parts {
                        parts: vec![ContentPart::ToolCall {
                            tool_call: ToolCall {
                                id: "nested".to_string(),
                                name: "unsupported".to_string(),
                                arguments_json: json!({"x": 1}),
                            },
                        }],
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let err = encode_anthropic_request(&req).expect_err("non-text tool_result should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(
        err.to_string()
            .contains("tool_result parts content must contain only text parts")
    );
}

#[test]
fn test_encode_tool_result_json_coerces_to_text_block() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "call the tool".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "tool_1".to_string(),
                    content: ToolResultContent::Json {
                        value: json!({"b": 2, "a": 1}),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let encoded = encode_anthropic_request(&req).expect("encode should succeed");
    assert_eq!(
        encoded.body.pointer("/messages/2/content/0/content/0/text"),
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
fn test_encode_tool_result_uses_raw_provider_content_when_array() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "call the tool".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "tool_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "tool_1".to_string(),
                    content: ToolResultContent::Text {
                        text: "fallback".to_string(),
                    },
                    raw_provider_content: Some(json!([
                        {"type":"text","text":"from-raw"},
                        {"type":"text","text":"second"}
                    ])),
                },
            }],
        },
    ];

    let encoded = encode_anthropic_request(&req).expect("encode should succeed");
    assert_eq!(
        encoded.body.pointer("/messages/2/content/0/content/0/text"),
        Some(&json!("from-raw"))
    );
    assert_eq!(
        encoded.body.pointer("/messages/2/content/0/content/1/text"),
        Some(&json!("second"))
    );
}

#[test]
fn test_decode_anthropic_translator_category_contract() {
    let payload = AnthropicDecodeEnvelope {
        body: json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "I will call a tool."},
                {"type": "tool_use", "id": "call_1", "name": "lookup_weather", "input": {"city": "SF"}},
                {"type": "thinking", "thinking": "hidden chain"},
                {"type": "redacted_thinking"},
                {"type": "server_tool_use", "name": "web_search"}
            ],
            "usage": {
                "input_tokens": 10,
                "cache_creation_input_tokens": 2,
                "cache_read_input_tokens": 3,
                "output_tokens": 5
            }
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_anthropic_response(&payload).expect("decode should succeed");

    assert_eq!(decoded.provider, ProviderId::Anthropic);
    assert_eq!(decoded.model, "claude-sonnet-4-5");
    assert_eq!(decoded.finish_reason, FinishReason::ToolCalls);
    assert_eq!(decoded.usage.input_tokens, Some(15));
    assert_eq!(decoded.usage.cached_input_tokens, Some(3));
    assert_eq!(decoded.usage.output_tokens, Some(5));
    assert_eq!(decoded.usage.total_tokens, Some(20));
    assert_eq!(decoded.output.content.len(), 3);

    assert!(matches!(
        &decoded.output.content[0],
        ContentPart::Text { text } if text == "I will call a tool."
    ));
    assert!(matches!(
        &decoded.output.content[1],
        ContentPart::ToolCall { tool_call } if tool_call.id == "call_1"
    ));
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "unknown_content_block_mapped_to_text")
    );
    assert_eq!(decoded.output.structured_output, None);
}

#[test]
fn test_decode_stop_reason_mapping_matrix() {
    let cases = vec![
        ("end_turn", FinishReason::Stop),
        ("stop_sequence", FinishReason::Stop),
        ("max_tokens", FinishReason::Length),
        ("tool_use", FinishReason::ToolCalls),
        ("refusal", FinishReason::ContentFilter),
        ("pause_turn", FinishReason::Other),
        ("new_future_reason", FinishReason::Other),
    ];

    for (reason, expected) in cases {
        let payload = AnthropicDecodeEnvelope {
            body: json!({
                "role": "assistant",
                "model": "claude-sonnet-4-5",
                "stop_reason": reason,
                "content": [{"type":"text","text":"ok"}],
                "usage": {"input_tokens":1,"output_tokens":1}
            }),
            requested_response_format: ResponseFormat::Text,
        };

        let decoded = decode_anthropic_response(&payload).expect("decode should succeed");
        assert_eq!(decoded.finish_reason, expected);

        if reason == "new_future_reason" {
            assert!(
                decoded
                    .warnings
                    .iter()
                    .any(|warning| warning.code == "unknown_stop_reason")
            );
        }
    }
}

#[test]
fn test_decode_structured_output_success_and_failure() {
    let success = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [{"type":"text","text":"{\"a\":1}"}],
            "usage": {"input_tokens":1,"output_tokens":1}
        }),
        requested_response_format: ResponseFormat::JsonSchema {
            name: "obj".to_string(),
            schema: json!({"type":"object"}),
        },
    };

    let decoded = decode_anthropic_response(&success).expect("decode should succeed");
    assert_eq!(decoded.output.structured_output, Some(json!({"a": 1})));

    let failure = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "max_tokens",
            "content": [{"type":"text","text":"{\"a\":"}],
            "usage": {"input_tokens":1,"output_tokens":1}
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_anthropic_response(&failure).expect("decode should succeed");
    assert_eq!(decoded.output.structured_output, None);
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "structured_output_parse_failed")
    );
}

#[test]
fn test_decode_usage_missing_and_partial() {
    let missing_usage_payload = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [{"type":"text","text":"ok"}]
        }),
        requested_response_format: ResponseFormat::Text,
    };
    let missing_usage = decode_anthropic_response(&missing_usage_payload).expect("decode missing");
    assert_eq!(missing_usage.usage.input_tokens, None);
    assert!(
        missing_usage
            .warnings
            .iter()
            .any(|warning| warning.code == "usage_missing")
    );

    let partial_usage_payload = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [{"type":"text","text":"ok"}],
            "usage": {"input_tokens": 4}
        }),
        requested_response_format: ResponseFormat::Text,
    };
    let partial_usage = decode_anthropic_response(&partial_usage_payload).expect("decode partial");
    assert_eq!(partial_usage.usage.input_tokens, Some(4));
    assert_eq!(partial_usage.usage.output_tokens, None);
    assert!(
        partial_usage
            .warnings
            .iter()
            .any(|warning| warning.code == "usage_partial")
    );
}

#[test]
fn test_decode_empty_output_emits_warning() {
    let payload = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [],
            "usage": {"input_tokens": 1, "output_tokens": 0}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded = decode_anthropic_response(&payload).expect("decode should succeed");
    assert!(decoded.output.content.is_empty());
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "empty_output")
    );
}

#[test]
fn test_anthropic_translator_determinism_contract() {
    let req = base_request();
    let first_encode = encode_anthropic_request(&req).expect("encode should succeed");
    let second_encode = encode_anthropic_request(&req).expect("encode should succeed");
    assert_eq!(first_encode, second_encode);

    let payload = AnthropicDecodeEnvelope {
        body: json!({
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "stop_reason": "end_turn",
            "content": [{"type":"text","text":"done"}],
            "usage": {"input_tokens":1,"output_tokens":1}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let first_decode = decode_anthropic_response(&payload).expect("decode should succeed");
    let second_decode = decode_anthropic_response(&payload).expect("decode should succeed");
    assert_eq!(first_decode, second_decode);
}

#[test]
fn test_parse_anthropic_error_envelope_and_format() {
    let envelope = parse_anthropic_error_envelope(
        r#"{"type":"error","error":{"type":"rate_limit_error","message":"too fast"},"request_id":"req_1"}"#,
    )
    .expect("should parse envelope");

    assert_eq!(envelope.error_type.as_deref(), Some("rate_limit_error"));
    assert_eq!(envelope.message, "too fast");
    assert_eq!(envelope.request_id.as_deref(), Some("req_1"));

    let message = format_anthropic_error_message(&envelope);
    assert!(message.contains("anthropic error: too fast"));
    assert!(message.contains("type=rate_limit_error"));
}

#[test]
fn test_decode_anthropic_models_list_success_and_invalid_payload() {
    let capabilities = ProviderCapabilities {
        supports_tools: true,
        supports_structured_output: true,
        supports_thinking: true,
        supports_remote_discovery: true,
    };

    let payload = json!({
        "data": [
            {"id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5"},
            {"id": "claude-haiku-4-5"},
            {"id": "claude-sonnet-4-5"}
        ]
    });

    let models = decode_anthropic_models_list(&payload, &capabilities).expect("decode models");
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model_id, "claude-sonnet-4-5");
    assert_eq!(models[0].display_name.as_deref(), Some("Claude Sonnet 4.5"));
    assert!(models.iter().all(|model| model.supports_tools));
    assert!(models.iter().all(|model| model.supports_structured_output));

    let err = decode_anthropic_models_list(&json!({"object":"list"}), &capabilities)
        .expect_err("missing data should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}
