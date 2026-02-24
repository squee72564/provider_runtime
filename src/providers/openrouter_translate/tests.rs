use std::collections::BTreeMap;

use serde_json::json;

use super::{
    OpenRouterDecodeEnvelope, OpenRouterTranslateOptions, decode_openrouter_models_list,
    decode_openrouter_response, encode_openrouter_request, format_openrouter_error_message,
    parse_openrouter_error_envelope,
};
use crate::core::error::ProviderError;
use crate::core::types::{
    ContentPart, FinishReason, Message, MessageRole, ModelRef, ProviderId, ProviderRequest,
    ResponseFormat, ToolCall, ToolChoice, ToolDefinition, ToolResult,
};

fn base_request() -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(ProviderId::Openrouter),
            model_id: "openai/gpt-4o-mini".to_string(),
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
fn test_encode_openrouter_translator_category_contract() {
    let mut req = base_request();
    req.messages = vec![
        Message {
            role: MessageRole::System,
            content: vec![ContentPart::Text {
                text: "Return JSON only".to_string(),
            }],
        },
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "weather in sf".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: ToolCall {
                    id: "call_1".to_string(),
                    name: "lookup_weather".to_string(),
                    arguments_json: json!({"city":"SF", "units":"f"}),
                },
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: "call_1".to_string(),
                    content: vec![ContentPart::Text {
                        text: "{\"temp\":55}".to_string(),
                    }],
                },
            }],
        },
    ];
    req.tools = vec![ToolDefinition {
        name: "lookup_weather".to_string(),
        description: Some("Lookup weather".to_string()),
        parameters_schema: json!({"type":"object","properties":{"city":{"type":"string"}}}),
    }];
    req.tool_choice = ToolChoice::Specific {
        name: "lookup_weather".to_string(),
    };
    req.response_format = ResponseFormat::JsonSchema {
        name: "weather_schema".to_string(),
        schema: json!({"type":"object","properties":{"temp":{"type":"number"}}}),
    };
    req.temperature = Some(0.7);
    req.top_p = Some(0.9);
    req.max_output_tokens = Some(200);
    req.stop = vec!["DONE".to_string()];
    req.metadata
        .insert("trace_id".to_string(), "abc".to_string());

    let options = OpenRouterTranslateOptions {
        fallback_models: vec!["anthropic/claude-sonnet-4.5".to_string()],
        provider_preferences: Some(json!({"allow_fallbacks":true})),
        plugins: vec![json!({"id":"response-healing","enabled":true})],
        parallel_tool_calls: Some(false),
    };

    let encoded = encode_openrouter_request(&req, &options).expect("encode should succeed");
    assert_eq!(encoded.body.pointer("/stream"), Some(&json!(false)));
    assert_eq!(encoded.body.pointer("/model"), None);
    assert_eq!(
        encoded.body.pointer("/models/0"),
        Some(&json!("openai/gpt-4o-mini"))
    );
    assert_eq!(
        encoded.body.pointer("/models/1"),
        Some(&json!("anthropic/claude-sonnet-4.5"))
    );
    assert_eq!(
        encoded.body.pointer("/response_format/type"),
        Some(&json!("json_schema"))
    );
    assert_eq!(
        encoded.body.pointer("/tool_choice/type"),
        Some(&json!("function"))
    );
    assert_eq!(
        encoded.body.pointer("/max_completion_tokens"),
        Some(&json!(200))
    );
    assert_eq!(
        encoded.body.pointer("/parallel_tool_calls"),
        Some(&json!(false))
    );

    assert!(
        encoded
            .warnings
            .iter()
            .any(|warning| warning.code == "both_temperature_and_top_p_set")
    );
}

#[test]
fn test_decode_openrouter_translator_category_contract() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id": "chatcmpl_1",
            "object": "chat.completion",
            "created": 171,
            "model": "openai/gpt-4o-mini",
            "choices": [
                {
                    "index": 0,
                    "finish_reason": "tool_calls",
                    "message": {
                        "role": "assistant",
                        "content": "{\"ok\":true}",
                        "reasoning": "short rationale",
                        "tool_calls": [
                            {
                                "id":"call_1",
                                "type":"function",
                                "function":{"name":"lookup_weather","arguments":"{\"city\":\"SF\"}"}
                            }
                        ]
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 7,
                "total_tokens": 19,
                "prompt_tokens_details": { "cached_tokens": 2 },
                "completion_tokens_details": { "reasoning_tokens": 3 }
            }
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_openrouter_response(&payload).expect("decode should succeed");

    assert_eq!(decoded.provider, ProviderId::Openrouter);
    assert_eq!(decoded.model, "openai/gpt-4o-mini");
    assert_eq!(decoded.finish_reason, FinishReason::ToolCalls);
    assert_eq!(decoded.usage.input_tokens, Some(12));
    assert_eq!(decoded.usage.output_tokens, Some(7));
    assert_eq!(decoded.usage.total_tokens, Some(19));
    assert_eq!(decoded.usage.cached_input_tokens, Some(2));
    assert_eq!(decoded.usage.reasoning_tokens, Some(3));
    assert_eq!(decoded.output.structured_output, Some(json!({"ok": true})));

    assert_eq!(decoded.output.content.len(), 3);
    assert!(
        matches!(&decoded.output.content[0], ContentPart::Text { text } if text == "{\"ok\":true}")
    );
    assert!(
        matches!(&decoded.output.content[1], ContentPart::ToolCall { tool_call } if tool_call.id == "call_1")
    );
    assert!(
        matches!(&decoded.output.content[2], ContentPart::Thinking { provider, .. } if provider == &Some(ProviderId::Openrouter))
    );
}

#[test]
fn test_openrouter_translator_determinism_contract() {
    let req = base_request();
    let options = OpenRouterTranslateOptions::default();

    let first_encode = encode_openrouter_request(&req, &options).expect("encode should succeed");
    let second_encode = encode_openrouter_request(&req, &options).expect("encode should succeed");
    assert_eq!(first_encode, second_encode);

    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"chatcmpl_1",
            "object":"chat.completion",
            "created":123,
            "model":"openai/gpt-4o-mini",
            "choices":[{
                "index":0,
                "finish_reason":"stop",
                "message":{"role":"assistant","content":"done"}
            }],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let first_decode = decode_openrouter_response(&payload).expect("decode should succeed");
    let second_decode = decode_openrouter_response(&payload).expect("decode should succeed");
    assert_eq!(first_decode, second_decode);
}

#[test]
fn test_encode_provider_hint_mismatch_is_error() {
    let mut req = base_request();
    req.model.provider_hint = Some(ProviderId::Openai);

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("provider hint mismatch should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}

#[test]
fn test_encode_empty_model_is_error() {
    let mut req = base_request();
    req.model.model_id = "  ".to_string();

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("missing model should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}

#[test]
fn test_encode_stop_limit_is_enforced() {
    let mut req = base_request();
    req.stop = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("stop size should fail");
    assert!(err.to_string().contains("at most 4"));
}

#[test]
fn test_encode_metadata_bounds_validation() {
    let mut req = base_request();
    for index in 0..17 {
        req.metadata
            .insert(format!("k{index}"), format!("value-{index}"));
    }

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("metadata size should fail");
    assert!(err.to_string().contains("at most 16 entries"));
}

#[test]
fn test_encode_tool_choice_specific_requires_declared_tool() {
    let mut req = base_request();
    req.tools = vec![ToolDefinition {
        name: "real_tool".to_string(),
        description: None,
        parameters_schema: json!({"type":"object"}),
    }];
    req.tool_choice = ToolChoice::Specific {
        name: "missing_tool".to_string(),
    };

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("unknown specific tool must fail");
    assert!(err.to_string().contains("references unknown tool"));
}

#[test]
fn test_encode_tool_role_content_validation() {
    let mut req = base_request();
    req.messages = vec![Message {
        role: MessageRole::Tool,
        content: vec![ContentPart::Text {
            text: "bad".to_string(),
        }],
    }];

    let err = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect_err("tool role text should fail");
    assert!(err.to_string().contains("tool_result"));
}

#[test]
fn test_encode_assistant_tool_call_arguments_are_stable_json() {
    let mut req = base_request();
    req.messages = vec![Message {
        role: MessageRole::Assistant,
        content: vec![ContentPart::ToolCall {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "lookup".to_string(),
                arguments_json: json!({"b":2,"a":1}),
            },
        }],
    }];

    let encoded = encode_openrouter_request(&req, &OpenRouterTranslateOptions::default())
        .expect("encode should succeed");

    assert_eq!(
        encoded
            .body
            .pointer("/messages/0/tool_calls/0/function/arguments"),
        Some(&json!("{\"a\":1,\"b\":2}"))
    );
}

#[test]
fn test_decode_top_level_error_is_protocol_error() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "error": { "code": 400, "message": "bad request" }
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let err = decode_openrouter_response(&payload).expect_err("error body should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("openrouter error"));
}

#[test]
fn test_decode_invalid_tool_arguments_warn_and_preserve_raw() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"chatcmpl_1",
            "object":"chat.completion",
            "created":123,
            "model":"openai/gpt-4o-mini",
            "choices":[{
                "index":0,
                "finish_reason":"tool_calls",
                "message":{
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[{
                        "id":"call_bad",
                        "type":"function",
                        "function":{"name":"lookup","arguments":"{not-json"}
                    }]
                }
            }],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded = decode_openrouter_response(&payload).expect("decode should succeed");
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "tool_arguments_invalid_json")
    );
    assert!(matches!(
        &decoded.output.content[0],
        ContentPart::ToolCall { tool_call }
            if tool_call.arguments_json == json!("{not-json")
    ));
}

#[test]
fn test_decode_usage_missing_and_partial_warn() {
    let missing = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"1",
            "object":"chat.completion",
            "created":1,
            "model":"openai/gpt-4o-mini",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded_missing = decode_openrouter_response(&missing).expect("decode should succeed");
    assert!(
        decoded_missing
            .warnings
            .iter()
            .any(|warning| warning.code == "usage_missing")
    );

    let partial = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"1",
            "object":"chat.completion",
            "created":1,
            "model":"openai/gpt-4o-mini",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}],
            "usage":{"prompt_tokens":3}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded_partial = decode_openrouter_response(&partial).expect("decode should succeed");
    assert!(
        decoded_partial
            .warnings
            .iter()
            .any(|warning| warning.code == "usage_partial")
    );
}

#[test]
fn test_decode_structured_output_parse_failure_warns() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"1",
            "object":"chat.completion",
            "created":1,
            "model":"openai/gpt-4o-mini",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"not-json"}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }),
        requested_response_format: ResponseFormat::JsonObject,
    };

    let decoded = decode_openrouter_response(&payload).expect("decode should succeed");
    assert_eq!(decoded.output.structured_output, None);
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "structured_output_parse_failed")
    );
}

#[test]
fn test_decode_finish_reason_mapping_matrix() {
    let cases = vec![
        ("stop", FinishReason::Stop),
        ("length", FinishReason::Length),
        ("tool_calls", FinishReason::ToolCalls),
        ("content_filter", FinishReason::ContentFilter),
        ("new_reason", FinishReason::Other),
    ];

    for (finish_reason, expected) in cases {
        let payload = OpenRouterDecodeEnvelope {
            body: json!({
                "id":"1",
                "object":"chat.completion",
                "created":1,
                "model":"openai/gpt-4o-mini",
                "choices":[{"index":0,"finish_reason":finish_reason,"message":{"role":"assistant","content":"ok"}}],
                "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
            }),
            requested_response_format: ResponseFormat::Text,
        };

        let decoded = decode_openrouter_response(&payload).expect("decode should succeed");
        assert_eq!(decoded.finish_reason, expected);
    }
}

#[test]
fn test_decode_finish_reason_error_is_failure() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"1",
            "object":"chat.completion",
            "created":1,
            "model":"openai/gpt-4o-mini",
            "choices":[{"index":0,"finish_reason":"error","message":{"role":"assistant","content":""}}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let err = decode_openrouter_response(&payload).expect_err("finish_reason=error should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}

#[test]
fn test_decode_empty_output_warns() {
    let payload = OpenRouterDecodeEnvelope {
        body: json!({
            "id":"1",
            "object":"chat.completion",
            "created":1,
            "model":"openai/gpt-4o-mini",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":null}}],
            "usage":{"prompt_tokens":1,"completion_tokens":0,"total_tokens":1}
        }),
        requested_response_format: ResponseFormat::Text,
    };

    let decoded = decode_openrouter_response(&payload).expect("decode should succeed");
    assert!(decoded.output.content.is_empty());
    assert!(
        decoded
            .warnings
            .iter()
            .any(|warning| warning.code == "empty_output")
    );
}

#[test]
fn test_parse_openrouter_error_envelope_and_format() {
    let envelope = parse_openrouter_error_envelope(
        r#"{"error":{"message":"No cookie auth credentials found","code":401}}"#,
    )
    .expect("should parse");

    assert_eq!(envelope.code, Some(401));
    assert_eq!(envelope.message, "No cookie auth credentials found");

    let message = format_openrouter_error_message(&envelope);
    assert!(message.contains("openrouter error"));
    assert!(message.contains("code=401"));
}

#[test]
fn test_decode_openrouter_models_list_success_and_invalid_payload() {
    let models = decode_openrouter_models_list(&json!({
        "data": [
            {
                "id":"openai/gpt-4o-mini",
                "name":"GPT-4o mini",
                "context_length": 128000,
                "top_provider":{"context_length": 128000, "max_completion_tokens": 4096},
                "supported_parameters": ["temperature", "tools", "response_format"]
            },
            {
                "id":"openai/gpt-4o-mini",
                "name":"GPT-4o mini duplicate",
                "supported_parameters": ["temperature"]
            },
            {
                "id":"some/old-model",
                "name":"Old model",
                "supported_parameters": ["temperature"]
            }
        ]
    }))
    .expect("decode should succeed");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model_id, "openai/gpt-4o-mini");
    assert_eq!(models[0].display_name.as_deref(), Some("GPT-4o mini"));
    assert_eq!(models[0].context_window, Some(128000));
    assert_eq!(models[0].max_output_tokens, Some(4096));
    assert!(models[0].supports_tools);
    assert!(models[0].supports_structured_output);

    assert_eq!(models[1].model_id, "some/old-model");
    assert!(!models[1].supports_tools);
    assert!(!models[1].supports_structured_output);

    let err = decode_openrouter_models_list(&json!({"object":"list"}))
        .expect_err("missing data should fail");
    assert!(matches!(err, ProviderError::Protocol { .. }));
}
