use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use provider_runtime::core::error::ProviderError;
use provider_runtime::core::traits::ProviderAdapter;
use provider_runtime::core::types::{
    AdapterContext, ContentPart, DiscoveryOptions, FinishReason, ProviderId, ProviderRequest,
    ProviderResponse,
};
use provider_runtime::providers::openrouter::{OpenRouterAdapter, OpenRouterAdapterOptions};
use serde_json::{Value, json};

const FIXTURE_ROOT: &str = "tests/fixtures/openrouter";

const ENCODE_FIXTURES: &[&str] = &[
    "encode/minimal_text_request.json",
    "encode/multi_message_conversation.json",
    "encode/tools_choice_none.json",
    "encode/tools_choice_auto.json",
    "encode/response_format_text.json",
    "encode/response_format_json_object.json",
    "encode/response_format_json_schema.json",
    "encode/controls_absent.json",
    "encode/controls_present_temperature_only.json",
    "encode/controls_present_top_p_only.json",
    "encode/controls_present_max_output_tokens.json",
    "encode/controls_present_metadata.json",
    "encode/controls_present_stop_present.json",
    "encode/unsupported_tool_choice_required.json",
    "encode/unsupported_tool_choice_specific.json",
];

const DECODE_FIXTURES: &[&str] = &[
    "decode/text_only_completed.json",
    "decode/tool_call_completed.json",
    "decode/structured_output_completed.json",
    "decode/usage_full.json",
    "decode/usage_partial.json",
    "decode/usage_absent.json",
    "decode/finish_reason_stop.json",
    "decode/finish_reason_length.json",
    "decode/finish_reason_tool_calls.json",
    "decode/finish_reason_content_filter.json",
    "decode/finish_reason_unknown.json",
];

const ERROR_FIXTURES: &[&str] = &[
    "errors/error_envelope_protocol_mapping.json",
    "errors/status_non_auth_mapping.json",
    "errors/malformed_payload_non_object.json",
    "errors/malformed_payload_missing_choices.json",
    "errors/malformed_payload_invalid_tool_calls_shape.json",
    "errors/unsupported_intent_tool_choice_required.json",
    "errors/unsupported_intent_tool_choice_specific.json",
];

const DETERMINISM_FIXTURES: &[&str] = &[
    "determinism/determinism_encode_input.json",
    "determinism/determinism_decode_payload.json",
];

#[derive(Debug, Clone)]
struct MockResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl MockResponse {
    fn json(body: &str) -> Self {
        Self {
            status_code: 200,
            headers: Vec::new(),
            body: body.to_string(),
        }
    }

    fn with_status(status_code: u16, headers: Vec<(String, String)>, body: &str) -> Self {
        Self {
            status_code,
            headers,
            body: body.to_string(),
        }
    }
}

struct MockServer {
    addr: std::net::SocketAddr,
    captured_requests: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        listener
            .set_nonblocking(false)
            .expect("configure blocking listener");
        let addr = listener.local_addr().expect("listener addr");

        let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
        let captured_requests = Arc::new(Mutex::new(Vec::new()));

        let queue_clone = Arc::clone(&queue);
        let captured_clone = Arc::clone(&captured_requests);

        let handle = thread::spawn(move || {
            loop {
                let next_response = {
                    let mut queue = queue_clone.lock().expect("queue lock");
                    queue.pop_front()
                };

                let Some(response) = next_response else {
                    break;
                };

                let (mut stream, _) = listener.accept().expect("accept connection");
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .expect("set stream timeout");

                let request = read_http_request_with_body(&mut stream);
                captured_clone.lock().expect("capture lock").push(request);

                let response_text =
                    build_http_response(response.status_code, &response.headers, &response.body);
                stream
                    .write_all(response_text.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
            }
        });

        Self {
            addr,
            captured_requests,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn captured_request_bodies(&self) -> Vec<Value> {
        self.captured_requests
            .lock()
            .expect("capture lock")
            .iter()
            .map(|raw_request| {
                let body = raw_request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body)
                    .unwrap_or_default();
                serde_json::from_str(body).expect("request body should be valid JSON")
            })
            .collect()
    }

    fn shutdown(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock server");
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn fixture_path(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_ROOT)
        .join(path)
}

fn load_fixture_str(path: &str) -> String {
    let abs = fixture_path(path);
    fs::read_to_string(&abs)
        .unwrap_or_else(|error| panic!("failed reading {}: {error}", abs.display()))
}

fn load_fixture_json(path: &str) -> Value {
    let raw = load_fixture_str(path);
    serde_json::from_str(&raw).unwrap_or_else(|error| panic!("failed parsing {path}: {error}"))
}

fn request_fixture(path: &str) -> ProviderRequest {
    let raw = load_fixture_str(path);
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("failed parsing canonical request fixture {path}: {error}"))
}

fn default_success_payload() -> &'static str {
    r#"{
      "id": "chatcmpl_success_1",
      "object": "chat.completion",
      "created": 1772173000,
      "model": "openai/gpt-5-mini",
      "choices": [
        {
          "index": 0,
          "finish_reason": "stop",
          "message": {
            "role": "assistant",
            "content": "ok"
          }
        }
      ],
      "usage": {
        "prompt_tokens": 1,
        "completion_tokens": 1,
        "total_tokens": 2,
        "prompt_tokens_details": {
          "cached_tokens": 0
        }
      }
    }"#
}

fn openrouter_adapter(base_url: String) -> OpenRouterAdapter {
    let options = OpenRouterAdapterOptions {
        fallback_models: vec!["openai/gpt-5-nano".to_string()],
        ..Default::default()
    };
    OpenRouterAdapter::with_base_url_and_options(Some("test-key".to_string()), base_url, options)
        .expect("create adapter")
}

fn assert_warning_codes(response: &ProviderResponse, expected_codes: &[&str]) {
    let actual_codes = response
        .warnings
        .iter()
        .map(|warning| warning.code.as_str())
        .collect::<Vec<_>>();
    for expected in expected_codes {
        assert!(
            actual_codes.contains(expected),
            "missing warning code `{expected}` in {actual_codes:?}"
        );
    }
}

fn assert_finish_reason(response: &ProviderResponse, expected: FinishReason) {
    assert_eq!(response.finish_reason, expected);
}

fn assert_usage_fields(
    response: &ProviderResponse,
    input: Option<u64>,
    output: Option<u64>,
    total: Option<u64>,
    cached: Option<u64>,
) {
    assert_eq!(response.usage.input_tokens, input);
    assert_eq!(response.usage.output_tokens, output);
    assert_eq!(response.usage.total_tokens, total);
    assert_eq!(response.usage.cached_input_tokens, cached);
}

fn assert_tool_call_content(response: &ProviderResponse, id: &str, name: &str) {
    assert!(matches!(
        response.output.content.iter().find(|part| matches!(part, ContentPart::ToolCall { .. })),
        Some(ContentPart::ToolCall { tool_call }) if tool_call.id == id && tool_call.name == name
    ));
}

fn assert_structured_output(response: &ProviderResponse, expected: Value) {
    assert_eq!(response.output.structured_output, Some(expected));
}

fn assert_fixture_exists(path: &str) {
    let abs = fixture_path(path);
    assert!(abs.exists(), "expected fixture to exist: {}", abs.display());
}

fn assert_error_fingerprint(err: &ProviderError) -> String {
    match err {
        ProviderError::Protocol {
            provider,
            model,
            request_id,
            message,
        } => format!("protocol:{provider:?}:{model:?}:{request_id:?}:{message}"),
        ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            message,
        } => format!("status:{provider:?}:{model:?}:{status_code}:{request_id:?}:{message}"),
        ProviderError::CredentialsRejected {
            provider,
            request_id,
            message,
        } => format!("credentials_rejected:{provider:?}:{request_id:?}:{message}"),
        ProviderError::Transport {
            provider,
            request_id,
            message,
        } => format!("transport:{provider:?}:{request_id:?}:{message}"),
        ProviderError::Serialization {
            provider,
            model,
            request_id,
            message,
        } => format!("serialization:{provider:?}:{model:?}:{request_id:?}:{message}"),
    }
}

#[tokio::test]
async fn test_openrouter_encode_fixture_contract() {
    let request_cases = vec![
        (
            "encode/minimal_text_request.json",
            json!({
                "messages_len": 1,
                "first_role": "user"
            }),
        ),
        (
            "encode/multi_message_conversation.json",
            json!({
                "messages_len": 2,
                "first_role": "system",
                "second_role": "user"
            }),
        ),
        (
            "encode/tools_choice_none.json",
            json!({"tool_choice": "none", "tool_len": 1}),
        ),
        (
            "encode/tools_choice_auto.json",
            json!({"tool_choice": "auto", "tool_len": 1}),
        ),
        (
            "encode/response_format_text.json",
            json!({"response_format_absent": true}),
        ),
        (
            "encode/response_format_json_object.json",
            json!({"response_format_type": "json_object"}),
        ),
        (
            "encode/response_format_json_schema.json",
            json!({"response_format_type": "json_schema", "schema_name": "weather_schema"}),
        ),
        (
            "encode/controls_absent.json",
            json!({"temperature_absent": true, "top_p_absent": true, "max_completion_tokens_absent": true, "metadata_absent": true, "stop_absent": true}),
        ),
        (
            "encode/controls_present_temperature_only.json",
            json!({"temperature": 0.2}),
        ),
        (
            "encode/controls_present_top_p_only.json",
            json!({"top_p": 0.7}),
        ),
        (
            "encode/controls_present_max_output_tokens.json",
            json!({"max_completion_tokens": 64}),
        ),
        (
            "encode/controls_present_metadata.json",
            json!({"metadata_trace_id": "fixture-1", "metadata_tenant": "acme"}),
        ),
        (
            "encode/controls_present_stop_present.json",
            json!({"stop_0": "DONE"}),
        ),
    ];

    for (fixture, expected) in request_cases {
        let mut server = MockServer::start(vec![MockResponse::json(default_success_payload())]);
        let adapter = openrouter_adapter(server.url());
        let request = request_fixture(fixture);

        adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect("run should succeed");

        server.shutdown();
        let bodies = server.captured_request_bodies();
        assert_eq!(bodies.len(), 1, "fixture {fixture}");
        let body = &bodies[0];

        assert_eq!(body.pointer("/stream"), Some(&json!(false)));
        assert_eq!(body.pointer("/models/0"), Some(&json!("openai/gpt-5-mini")));
        assert_eq!(body.pointer("/models/1"), Some(&json!("openai/gpt-5-nano")));

        if let Some(choice) = expected.get("tool_choice").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/tool_choice"),
                Some(&json!(choice)),
                "fixture {fixture}"
            );
        }
        if let Some(tool_len) = expected.get("tool_len").and_then(Value::as_u64) {
            let tools = body
                .get("tools")
                .and_then(Value::as_array)
                .expect("tools should be array");
            assert_eq!(tools.len() as u64, tool_len, "fixture {fixture}");
        }

        if expected
            .get("response_format_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("response_format").is_none(), "fixture {fixture}");
        }
        if let Some(format_type) = expected.get("response_format_type").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/response_format/type"),
                Some(&json!(format_type)),
                "fixture {fixture}"
            );
        }
        if let Some(schema_name) = expected.get("schema_name").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/response_format/json_schema/name"),
                Some(&json!(schema_name)),
                "fixture {fixture}"
            );
        }

        if let Some(messages_len) = expected.get("messages_len").and_then(Value::as_u64) {
            let messages = body
                .get("messages")
                .and_then(Value::as_array)
                .expect("messages should be array");
            assert_eq!(messages.len() as u64, messages_len, "fixture {fixture}");
        }
        if let Some(first_role) = expected.get("first_role").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/messages/0/role"),
                Some(&json!(first_role)),
                "fixture {fixture}"
            );
        }
        if let Some(second_role) = expected.get("second_role").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/messages/1/role"),
                Some(&json!(second_role)),
                "fixture {fixture}"
            );
        }

        if expected
            .get("temperature_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("temperature").is_none(), "fixture {fixture}");
        }
        if expected
            .get("top_p_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("top_p").is_none(), "fixture {fixture}");
        }
        if expected
            .get("max_completion_tokens_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(
                body.get("max_completion_tokens").is_none(),
                "fixture {fixture}"
            );
        }
        if expected
            .get("metadata_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("metadata").is_none(), "fixture {fixture}");
        }
        if expected
            .get("stop_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("stop").is_none(), "fixture {fixture}");
        }

        if let Some(temperature) = expected.get("temperature").and_then(Value::as_f64) {
            let actual = body
                .pointer("/temperature")
                .and_then(Value::as_f64)
                .expect("temperature should be numeric");
            assert!((actual - temperature).abs() < 1e-6, "fixture {fixture}");
        }
        if let Some(top_p) = expected.get("top_p").and_then(Value::as_f64) {
            let actual = body
                .pointer("/top_p")
                .and_then(Value::as_f64)
                .expect("top_p should be numeric");
            assert!((actual - top_p).abs() < 1e-6, "fixture {fixture}");
        }
        if let Some(max_completion_tokens) = expected
            .get("max_completion_tokens")
            .and_then(Value::as_u64)
        {
            assert_eq!(
                body.pointer("/max_completion_tokens"),
                Some(&json!(max_completion_tokens)),
                "fixture {fixture}"
            );
        }
        if let Some(stop_0) = expected.get("stop_0").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/stop/0"),
                Some(&json!(stop_0)),
                "fixture {fixture}"
            );
        }
        if let Some(trace_id) = expected.get("metadata_trace_id").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/metadata/trace_id"),
                Some(&json!(trace_id)),
                "fixture {fixture}"
            );
        }
        if let Some(tenant) = expected.get("metadata_tenant").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/metadata/tenant"),
                Some(&json!(tenant)),
                "fixture {fixture}"
            );
        }
    }

    for fixture in [
        "encode/unsupported_tool_choice_required.json",
        "encode/unsupported_tool_choice_specific.json",
    ] {
        let request = request_fixture(fixture);
        let adapter =
            OpenRouterAdapter::with_base_url(Some("test-key".to_string()), "http://127.0.0.1:1")
                .expect("create adapter");

        let err = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect_err("unsupported tool-choice intent should fail deterministically");

        match err {
            ProviderError::Protocol {
                provider, message, ..
            } => {
                assert_eq!(provider, ProviderId::Openrouter);
                assert!(message.contains("requires at least one tool definition"));
            }
            other => panic!("expected protocol error for {fixture}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_openrouter_decode_fixture_contract() {
    let decode_cases = vec![
        (
            "decode/text_only_completed.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(50), Some(261), Some(311), Some(0))),
            Some("Feeling ready and helpful today."),
            None,
            None,
        ),
        (
            "decode/tool_call_completed.json",
            "encode/tools_choice_auto.json",
            FinishReason::ToolCalls,
            Some((Some(60), Some(86), Some(146), Some(0))),
            None,
            Some(("call_XiXyHd1CVfi2637JmTd4nDc7", "get_weather")),
            None,
        ),
        (
            "decode/structured_output_completed.json",
            "encode/response_format_json_object.json",
            FinishReason::Stop,
            Some((Some(18), Some(7), Some(25), Some(0))),
            Some("{\"ok\":true,\"city\":\"SF\"}"),
            None,
            Some(json!({"ok": true, "city": "SF"})),
        ),
        (
            "decode/usage_full.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(50), Some(261), Some(311), Some(0))),
            Some("Feeling ready and helpful today."),
            None,
            None,
        ),
        (
            "decode/usage_partial.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(11), None, None, None)),
            Some("usage partial"),
            None,
            None,
        ),
        (
            "decode/usage_absent.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((None, None, None, None)),
            Some("usage absent"),
            None,
            None,
        ),
        (
            "decode/finish_reason_stop.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(1), Some(1), Some(2), None)),
            Some("stop"),
            None,
            None,
        ),
        (
            "decode/finish_reason_length.json",
            "encode/minimal_text_request.json",
            FinishReason::Length,
            Some((Some(1), Some(2), Some(3), None)),
            Some("length"),
            None,
            None,
        ),
        (
            "decode/finish_reason_tool_calls.json",
            "encode/tools_choice_auto.json",
            FinishReason::ToolCalls,
            Some((Some(2), Some(3), Some(5), None)),
            None,
            Some(("call_abc", "lookup_weather")),
            None,
        ),
        (
            "decode/finish_reason_content_filter.json",
            "encode/minimal_text_request.json",
            FinishReason::ContentFilter,
            Some((Some(1), Some(1), Some(2), None)),
            Some("filtered"),
            None,
            None,
        ),
        (
            "decode/finish_reason_unknown.json",
            "encode/minimal_text_request.json",
            FinishReason::Other,
            Some((Some(1), Some(1), Some(2), None)),
            Some("unknown"),
            None,
            None,
        ),
    ];

    for (
        decode_fixture,
        request_fixture_path,
        finish_reason,
        usage,
        expected_text,
        tool_call,
        structured,
    ) in decode_cases
    {
        let payload = load_fixture_str(decode_fixture);
        let mut server = MockServer::start(vec![MockResponse::json(&payload)]);
        let adapter = openrouter_adapter(server.url());
        let request = request_fixture(request_fixture_path);

        let response = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect("run should succeed");

        assert_finish_reason(&response, finish_reason);
        if let Some((input, output, total, cached)) = usage {
            assert_usage_fields(&response, input, output, total, cached);
        }
        if let Some(text) = expected_text {
            assert!(matches!(
                response.output.content.first(),
                Some(ContentPart::Text { text: actual }) if actual == text
            ));
        }
        if let Some((id, name)) = tool_call {
            assert_tool_call_content(&response, id, name);
        }
        if let Some(expected_structured) = structured {
            assert_structured_output(&response, expected_structured);
        }

        if decode_fixture.ends_with("usage_partial.json") {
            assert_warning_codes(&response, &["usage_partial"]);
        }
        if decode_fixture.ends_with("usage_absent.json") {
            assert_warning_codes(&response, &["usage_missing"]);
        }
        if decode_fixture.ends_with("finish_reason_unknown.json") {
            assert_warning_codes(&response, &["unknown_finish_reason"]);
        }

        server.shutdown();
    }
}

#[test]
fn test_openrouter_fixture_category_matrix_coverage() {
    let canonical_request_categories: &[(&str, &[&str])] = &[
        (
            "minimal text request",
            &["encode/minimal_text_request.json"],
        ),
        (
            "multi-message conversation",
            &["encode/multi_message_conversation.json"],
        ),
        (
            "tools with each tool-choice mode",
            &[
                "encode/tools_choice_none.json",
                "encode/tools_choice_auto.json",
                "encode/unsupported_tool_choice_required.json",
                "encode/unsupported_tool_choice_specific.json",
            ],
        ),
        (
            "response formats",
            &[
                "encode/response_format_text.json",
                "encode/response_format_json_object.json",
                "encode/response_format_json_schema.json",
            ],
        ),
        (
            "optional controls present and absent",
            &[
                "encode/controls_absent.json",
                "encode/controls_present_temperature_only.json",
                "encode/controls_present_top_p_only.json",
                "encode/controls_present_max_output_tokens.json",
                "encode/controls_present_metadata.json",
                "encode/controls_present_stop_present.json",
            ],
        ),
    ];

    let provider_response_categories: &[(&str, &[&str])] = &[
        (
            "text-only assistant output",
            &["decode/text_only_completed.json"],
        ),
        ("tool-call output", &["decode/tool_call_completed.json"]),
        (
            "structured output present",
            &["decode/structured_output_completed.json"],
        ),
        (
            "usage fields partial/full/absent",
            &[
                "decode/usage_full.json",
                "decode/usage_partial.json",
                "decode/usage_absent.json",
            ],
        ),
        (
            "finish reason normalization",
            &[
                "decode/finish_reason_stop.json",
                "decode/finish_reason_length.json",
                "decode/finish_reason_tool_calls.json",
                "decode/finish_reason_content_filter.json",
                "decode/finish_reason_unknown.json",
            ],
        ),
    ];

    let error_edge_categories: &[(&str, &[&str])] = &[
        (
            "protocol error payload mapping",
            &[
                "errors/error_envelope_protocol_mapping.json",
                "errors/status_non_auth_mapping.json",
            ],
        ),
        (
            "unsupported canonical intent",
            &[
                "errors/unsupported_intent_tool_choice_required.json",
                "errors/unsupported_intent_tool_choice_specific.json",
            ],
        ),
        (
            "malformed payload decode failures",
            &[
                "errors/malformed_payload_non_object.json",
                "errors/malformed_payload_missing_choices.json",
                "errors/malformed_payload_invalid_tool_calls_shape.json",
            ],
        ),
    ];

    let determinism_categories: &[(&str, &[&str])] = &[
        (
            "stable encode output for identical canonical input",
            &["determinism/determinism_encode_input.json"],
        ),
        (
            "stable decode output for identical provider payload",
            &["determinism/determinism_decode_payload.json"],
        ),
        (
            "stable warning/error behavior",
            &[
                "errors/malformed_payload_missing_choices.json",
                "errors/unsupported_intent_tool_choice_required.json",
            ],
        ),
    ];

    for (group_name, groups) in [
        ("canonical request categories", canonical_request_categories),
        ("provider response categories", provider_response_categories),
        ("error/edge categories", error_edge_categories),
        ("determinism categories", determinism_categories),
    ] {
        assert!(!groups.is_empty(), "{group_name} must not be empty");
        for (category, fixture_ids) in groups {
            assert!(
                !fixture_ids.is_empty(),
                "category `{category}` under `{group_name}` has no fixtures"
            );
            for fixture in *fixture_ids {
                assert_fixture_exists(fixture);
            }
        }
    }

    for fixture in ENCODE_FIXTURES {
        assert_fixture_exists(fixture);
    }
    for fixture in DECODE_FIXTURES {
        assert_fixture_exists(fixture);
    }
    for fixture in ERROR_FIXTURES {
        assert_fixture_exists(fixture);
    }
    for fixture in DETERMINISM_FIXTURES {
        assert_fixture_exists(fixture);
    }
}

#[tokio::test]
async fn test_openrouter_contract_non_2xx_auth_maps_to_credentials_rejected() {
    let error_body = load_fixture_str("errors/error_envelope_protocol_mapping.json");
    let mut server = MockServer::start(vec![MockResponse::with_status(
        401,
        vec![("x-request-id".to_string(), "req-contract-auth".to_string())],
        &error_body,
    )]);
    let adapter = openrouter_adapter(server.url());

    let request = request_fixture("encode/minimal_text_request.json");
    let err = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("auth error should fail");

    match err {
        ProviderError::CredentialsRejected {
            provider,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openrouter);
            assert_eq!(request_id, Some("req-contract-auth".to_string()));
            assert!(message.contains("openrouter error"));
            assert!(message.contains("Missing Authentication header"));
            assert!(message.contains("code=401"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_non_2xx_non_auth_maps_to_status() {
    let error_body = load_fixture_str("errors/status_non_auth_mapping.json");
    let response = MockResponse::with_status(
        429,
        vec![("x-request-id".to_string(), "req-contract-rate".to_string())],
        &error_body,
    );
    let mut server = MockServer::start(vec![response.clone(), response.clone(), response]);
    let adapter = openrouter_adapter(server.url());

    let request = request_fixture("encode/minimal_text_request.json");
    let err = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("status error should fail");

    match err {
        ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openrouter);
            assert_eq!(model, Some("openai/gpt-5-mini".to_string()));
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-contract-rate".to_string()));
            assert!(message.contains("openrouter error"));
            assert!(message.contains("not a valid model ID"));
            assert!(message.contains("code=400"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_request_id_header_behavior_is_deterministic() {
    let error_body = load_fixture_str("errors/status_non_auth_mapping.json");
    let request = request_fixture("encode/minimal_text_request.json");

    let mut header_server = MockServer::start(vec![MockResponse::with_status(
        400,
        vec![("x-request-id".to_string(), "req-header-win".to_string())],
        &error_body,
    )]);
    let adapter = openrouter_adapter(header_server.url());
    let err = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("status should fail");

    match err {
        ProviderError::Status { request_id, .. } => {
            assert_eq!(request_id, Some("req-header-win".to_string()));
        }
        other => panic!("expected status error with header request id, got {other:?}"),
    }
    header_server.shutdown();

    let mut no_header_server = MockServer::start(vec![MockResponse::with_status(
        400,
        Vec::new(),
        &error_body,
    )]);
    let adapter = openrouter_adapter(no_header_server.url());
    let err = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("status should fail");

    match err {
        ProviderError::Status { request_id, .. } => {
            assert_eq!(request_id, None);
        }
        other => panic!("expected status error without request id, got {other:?}"),
    }
    no_header_server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_discovery_models_mapping() {
    let mut server = MockServer::start(vec![MockResponse::json(
        r#"{
            "data":[
                {
                  "id":"openai/gpt-5-mini",
                  "name":"GPT-5 mini",
                  "top_provider": {"context_length": 400000, "max_completion_tokens": 4096},
                  "supported_parameters": ["temperature", "tools", "response_format"]
                },
                {
                  "id":"openai/gpt-5-mini",
                  "name":"GPT-5 mini dup",
                  "supported_parameters": ["temperature"]
                },
                {
                  "id":"openai/gpt-5-nano",
                  "supported_parameters": ["temperature"]
                }
            ]
        }"#,
    )]);
    let adapter = openrouter_adapter(server.url());

    let models = adapter
        .discover_models(
            &DiscoveryOptions {
                remote: true,
                include_provider: Vec::new(),
                refresh_cache: true,
            },
            &AdapterContext::default(),
        )
        .await
        .expect("discover should succeed");

    assert_eq!(models.len(), 2);
    assert_eq!(models[0].model_id, "openai/gpt-5-mini");
    assert_eq!(models[1].model_id, "openai/gpt-5-nano");
    assert!(
        models
            .iter()
            .all(|model| model.provider == ProviderId::Openrouter)
    );
    assert_eq!(models[0].display_name, Some("GPT-5 mini".to_string()));
    assert_eq!(models[0].context_window, Some(400000));
    assert_eq!(models[0].max_output_tokens, Some(4096));
    assert!(models[0].supports_tools);
    assert!(models[0].supports_structured_output);

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_malformed_payload_decode_failures_are_protocol() {
    let malformed_fixtures = vec![
        "errors/malformed_payload_non_object.json",
        "errors/malformed_payload_missing_choices.json",
        "errors/malformed_payload_invalid_tool_calls_shape.json",
    ];

    let parsed_non_object = load_fixture_json("errors/malformed_payload_non_object.json");
    assert!(parsed_non_object.is_string());

    for fixture in malformed_fixtures {
        let response_body = load_fixture_str(fixture);
        let mut server = MockServer::start(vec![MockResponse::json(&response_body)]);
        let adapter = openrouter_adapter(server.url());

        let request = request_fixture("encode/minimal_text_request.json");
        let err = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect_err("malformed payload should fail");

        match err {
            ProviderError::Protocol { provider, .. } => {
                assert_eq!(provider, ProviderId::Openrouter, "fixture {fixture}");
            }
            other => panic!("expected protocol error for {fixture}, got {other:?}"),
        }

        server.shutdown();
    }
}

#[tokio::test]
async fn test_openrouter_contract_unsupported_intent_is_deterministic() {
    for fixture in [
        "errors/unsupported_intent_tool_choice_required.json",
        "errors/unsupported_intent_tool_choice_specific.json",
    ] {
        let request = request_fixture(fixture);

        let adapter =
            OpenRouterAdapter::with_base_url(Some("test-key".to_string()), "http://127.0.0.1:1")
                .expect("create adapter");

        let first = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect_err("first run should fail");
        let second = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect_err("second run should fail");

        assert_eq!(
            assert_error_fingerprint(&first),
            assert_error_fingerprint(&second),
            "fixture {fixture}"
        );
    }
}

#[tokio::test]
async fn test_openrouter_contract_encode_is_deterministic_for_identical_input() {
    let request = request_fixture("determinism/determinism_encode_input.json");
    let decode_payload = load_fixture_str("decode/finish_reason_stop.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&decode_payload),
        MockResponse::json(&decode_payload),
    ]);
    let adapter = openrouter_adapter(server.url());

    let first = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect("first run should succeed");
    let second = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect("second run should succeed");

    let captured = server.captured_request_bodies();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0], captured[1]);
    assert_eq!(first.warnings, second.warnings);

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_decode_is_deterministic_for_identical_payload() {
    let request = request_fixture("encode/response_format_json_object.json");
    let payload = load_fixture_str("determinism/determinism_decode_payload.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&payload),
        MockResponse::json(&payload),
    ]);
    let adapter = openrouter_adapter(server.url());

    let first = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect("first run should succeed");
    let second = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect("second run should succeed");

    assert_eq!(first, second);

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_contract_malformed_failure_is_deterministic() {
    let request = request_fixture("encode/minimal_text_request.json");
    let malformed = load_fixture_str("errors/malformed_payload_missing_choices.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&malformed),
        MockResponse::json(&malformed),
    ]);
    let adapter = openrouter_adapter(server.url());

    let first = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("first run should fail");
    let second = adapter
        .run(&request, &AdapterContext::default())
        .await
        .expect_err("second run should fail");

    assert_eq!(
        assert_error_fingerprint(&first),
        assert_error_fingerprint(&second)
    );

    server.shutdown();
}

fn read_http_request_with_body(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(bytes_read) => {
                request.extend_from_slice(&chunk[..bytes_read]);

                if let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            if name.eq_ignore_ascii_case("content-length") {
                                value.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    let total_required = header_end + 4 + content_length;
                    if request.len() >= total_required {
                        break;
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(error) => panic!("failed reading request: {error}"),
        }
    }

    String::from_utf8_lossy(&request).to_string()
}

fn build_http_response(status_code: u16, headers: &[(String, String)], body: &str) -> String {
    let mut rendered = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        status_code,
        status_reason(status_code),
        body.len(),
    );
    for (name, value) in headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    rendered.push_str("\r\n");
    rendered.push_str(body);
    rendered
}

fn status_reason(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Unknown",
    }
}
