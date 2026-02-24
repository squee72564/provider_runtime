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
            content: vec![
                ContentPart::Text {
                    text: "Return weather as JSON".to_string(),
                },
                ContentPart::Thinking {
                    text: "private".to_string(),
                    provider: Some(ProviderId::Anthropic),
                },
            ],
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
                    content: vec![ContentPart::Text {
                        text: "{\"temp\":55}".to_string(),
                    }],
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
    assert!(warning_codes.contains(&"dropped_thinking_on_encode"));
    assert!(warning_codes.contains(&"both_temperature_and_top_p_set"));
    assert!(warning_codes.contains(&"default_max_tokens_applied"));
    assert!(warning_codes.contains(&"dropped_unsupported_metadata_keys"));
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
    assert_eq!(decoded.output.content.len(), 5);

    assert!(matches!(
        &decoded.output.content[0],
        ContentPart::Text { text } if text == "I will call a tool."
    ));
    assert!(matches!(
        &decoded.output.content[1],
        ContentPart::ToolCall { tool_call } if tool_call.id == "call_1"
    ));
    assert!(matches!(
        &decoded.output.content[2],
        ContentPart::Thinking { provider, .. } if provider == &Some(ProviderId::Anthropic)
    ));
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "redacted_thinking_mapped")
    );
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
