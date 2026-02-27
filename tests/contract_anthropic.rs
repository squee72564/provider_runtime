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
use provider_runtime::providers::anthropic::AnthropicAdapter;
use serde_json::{Value, json};

const FIXTURE_ROOT: &str = "tests/fixtures/anthropic";

const ENCODE_FIXTURES: &[&str] = &[
    "encode/minimal_text_request.json",
    "encode/multi_message_conversation.json",
    "encode/tools_choice_none.json",
    "encode/tools_choice_auto.json",
    "encode/tools_choice_required.json",
    "encode/tools_choice_specific.json",
    "encode/response_format_text.json",
    "encode/response_format_json_object.json",
    "encode/response_format_json_schema.json",
    "encode/controls_absent.json",
    "encode/controls_present_temperature_only.json",
    "encode/controls_present_top_p_only.json",
    "encode/controls_present_max_output_tokens.json",
    "encode/controls_present_metadata.json",
    "encode/controls_present_stop_present.json",
];

const DECODE_FIXTURES: &[&str] = &[
    "decode/text_only_completed.json",
    "decode/tool_call_completed.json",
    "decode/structured_output_completed.json",
    "decode/usage_full.json",
    "decode/usage_partial.json",
    "decode/usage_absent.json",
    "decode/finish_reason_end_turn.json",
    "decode/finish_reason_max_tokens.json",
    "decode/finish_reason_tool_use.json",
    "decode/finish_reason_unknown.json",
];

const ERROR_FIXTURES: &[&str] = &[
    "errors/error_envelope_protocol_mapping.json",
    "errors/unsupported_intent_json_prefill.json",
    "errors/malformed_payload_non_object.json",
    "errors/malformed_payload_missing_stop_reason.json",
    "errors/malformed_payload_tool_use_non_object_input.json",
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

fn load_request_fixture(path: &str) -> ProviderRequest {
    let raw = load_fixture_str(path);
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("failed parsing canonical request fixture {path}: {error}"))
}

fn request_fixture(path: &str) -> ProviderRequest {
    load_request_fixture(path)
}

fn default_success_payload() -> &'static str {
    r#"{
        "model":"claude-sonnet-4-5-20250929",
        "id":"msg_success_1",
        "type":"message",
        "role":"assistant",
        "content":[{"type":"text","text":"ok"}],
        "stop_reason":"end_turn",
        "usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}
    }"#
}

fn anthropic_adapter(base_url: String) -> AnthropicAdapter {
    AnthropicAdapter::with_base_url(Some("test-key".to_string()), base_url).expect("create adapter")
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
        response.output.content.first(),
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
async fn test_anthropic_encode_fixture_contract() {
    let request_cases = vec![
        (
            "encode/minimal_text_request.json",
            json!({
                "tool_choice_type": "auto",
                "input_len": 1,
                "first_role": "user",
                "max_tokens": 1024
            }),
        ),
        (
            "encode/multi_message_conversation.json",
            json!({
                "input_len": 1,
                "first_role": "user",
                "system_len": 1,
                "max_tokens": 1024
            }),
        ),
        (
            "encode/tools_choice_none.json",
            json!({"tool_choice_type": "none", "tool_len": 1, "max_tokens": 1024}),
        ),
        (
            "encode/tools_choice_auto.json",
            json!({"tool_choice_type": "auto", "tool_len": 1, "max_tokens": 1024}),
        ),
        (
            "encode/tools_choice_required.json",
            json!({"tool_choice_type": "any", "tool_len": 1, "max_tokens": 1024}),
        ),
        (
            "encode/tools_choice_specific.json",
            json!({"tool_choice_type": "tool", "tool_choice_name": "calculator", "tool_len": 1, "max_tokens": 1024}),
        ),
        (
            "encode/response_format_text.json",
            json!({"output_config_absent": true, "max_tokens": 1024}),
        ),
        (
            "encode/response_format_json_object.json",
            json!({"output_format_type": "json_schema", "output_schema_type": "object", "max_tokens": 1024}),
        ),
        (
            "encode/response_format_json_schema.json",
            json!({"output_format_type": "json_schema", "output_schema_city_type": "string", "max_tokens": 1024}),
        ),
        (
            "encode/controls_absent.json",
            json!({"temperature_absent": true, "top_p_absent": true, "metadata_absent": true, "max_tokens": 1024}),
        ),
        (
            "encode/controls_present_temperature_only.json",
            json!({"temperature": 0.2, "max_tokens": 1024}),
        ),
        (
            "encode/controls_present_top_p_only.json",
            json!({"top_p": 0.7, "max_tokens": 1024}),
        ),
        (
            "encode/controls_present_max_output_tokens.json",
            json!({"max_tokens": 64}),
        ),
        (
            "encode/controls_present_metadata.json",
            json!({"metadata_user_id": "user-123", "metadata_trace_absent": true, "max_tokens": 1024}),
        ),
        (
            "encode/controls_present_stop_present.json",
            json!({"stop_0": "DONE", "max_tokens": 1024}),
        ),
    ];

    for (fixture, expected) in request_cases {
        let mut server = MockServer::start(vec![MockResponse::json(default_success_payload())]);
        let adapter = anthropic_adapter(server.url());
        let request = request_fixture(fixture);

        let response = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect("run should succeed");

        server.shutdown();
        let bodies = server.captured_request_bodies();
        assert_eq!(bodies.len(), 1, "fixture {fixture}");
        let body = &bodies[0];

        assert_eq!(
            body.pointer("/model"),
            Some(&json!("claude-sonnet-4-5-20250929"))
        );

        if let Some(choice_type) = expected.get("tool_choice_type").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/tool_choice/type"),
                Some(&json!(choice_type)),
                "fixture {fixture}"
            );
        }
        if let Some(choice_name) = expected.get("tool_choice_name").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/tool_choice/name"),
                Some(&json!(choice_name)),
                "fixture {fixture}"
            );
            assert_eq!(
                body.pointer("/tool_choice/disable_parallel_tool_use"),
                Some(&json!(true)),
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
        if let Some(output_format_type) = expected.get("output_format_type").and_then(Value::as_str)
        {
            assert_eq!(
                body.pointer("/output_config/format/type"),
                Some(&json!(output_format_type)),
                "fixture {fixture}"
            );
        }
        if let Some(output_schema_type) = expected.get("output_schema_type").and_then(Value::as_str)
        {
            assert_eq!(
                body.pointer("/output_config/format/schema/type"),
                Some(&json!(output_schema_type)),
                "fixture {fixture}"
            );
        }
        if let Some(output_schema_city_type) = expected
            .get("output_schema_city_type")
            .and_then(Value::as_str)
        {
            assert_eq!(
                body.pointer("/output_config/format/schema/properties/city/type"),
                Some(&json!(output_schema_city_type)),
                "fixture {fixture}"
            );
        }
        if let Some(input_len) = expected.get("input_len").and_then(Value::as_u64) {
            let input = body
                .get("messages")
                .and_then(Value::as_array)
                .expect("messages should be array");
            assert_eq!(input.len() as u64, input_len, "fixture {fixture}");
        }
        if let Some(first_role) = expected.get("first_role").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/messages/0/role"),
                Some(&json!(first_role)),
                "fixture {fixture}"
            );
        }
        if let Some(system_len) = expected.get("system_len").and_then(Value::as_u64) {
            let system = body
                .get("system")
                .and_then(Value::as_array)
                .expect("system should be array");
            assert_eq!(system.len() as u64, system_len, "fixture {fixture}");
        }

        if expected
            .get("output_config_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("output_config").is_none(), "fixture {fixture}");
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
            .get("metadata_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(body.get("metadata").is_none(), "fixture {fixture}");
        }
        if let Some(temperature) = expected.get("temperature").and_then(Value::as_f64) {
            let actual = body
                .pointer("/temperature")
                .and_then(Value::as_f64)
                .expect("temperature should be numeric");
            assert!(
                (actual - temperature).abs() < 1e-6,
                "fixture {fixture}: expected temperature {temperature}, got {actual}"
            );
        }
        if let Some(top_p) = expected.get("top_p").and_then(Value::as_f64) {
            let actual = body
                .pointer("/top_p")
                .and_then(Value::as_f64)
                .expect("top_p should be numeric");
            assert!(
                (actual - top_p).abs() < 1e-6,
                "fixture {fixture}: expected top_p {top_p}, got {actual}"
            );
        }
        if let Some(max_tokens) = expected.get("max_tokens").and_then(Value::as_u64) {
            assert_eq!(
                body.pointer("/max_tokens"),
                Some(&json!(max_tokens)),
                "fixture {fixture}"
            );
        }
        if let Some(user_id) = expected.get("metadata_user_id").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/metadata/user_id"),
                Some(&json!(user_id)),
                "fixture {fixture}"
            );
        }
        if expected
            .get("metadata_trace_absent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            assert!(
                body.pointer("/metadata/trace_id").is_none(),
                "fixture {fixture}"
            );
        }
        if let Some(stop_0) = expected.get("stop_0").and_then(Value::as_str) {
            assert_eq!(
                body.pointer("/stop_sequences/0"),
                Some(&json!(stop_0)),
                "fixture {fixture}"
            );
        }

        if fixture != "encode/controls_present_max_output_tokens.json" {
            assert_warning_codes(&response, &["default_max_tokens_applied"]);
        }
        if fixture == "encode/controls_present_metadata.json" {
            assert_warning_codes(&response, &["dropped_unsupported_metadata_keys"]);
        }
    }
}

#[tokio::test]
async fn test_anthropic_decode_fixture_contract() {
    let decode_cases = vec![
        (
            "decode/text_only_completed.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(591), Some(10), Some(601), Some(0))),
            Some("I'm ready to help you!"),
            None,
            None,
        ),
        (
            "decode/tool_call_completed.json",
            "encode/tools_choice_specific.json",
            FinishReason::ToolCalls,
            Some((Some(591), Some(60), Some(651), Some(0))),
            None,
            Some(("toolu_018riaCe1uSpr2Mbo4Jib6mq", "calculator")),
            None,
        ),
        (
            "decode/structured_output_completed.json",
            "encode/response_format_json_object.json",
            FinishReason::Stop,
            Some((Some(601), Some(18), Some(619), Some(0))),
            Some("{\"ok\":true,\"city\":\"SF\"}"),
            None,
            Some(json!({"ok": true, "city": "SF"})),
        ),
        (
            "decode/usage_full.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(694), Some(76), Some(770), Some(3))),
            Some("The answer is 5.414213562"),
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
            "decode/finish_reason_end_turn.json",
            "encode/minimal_text_request.json",
            FinishReason::Stop,
            Some((Some(1), Some(1), Some(2), None)),
            Some("completed"),
            None,
            None,
        ),
        (
            "decode/finish_reason_max_tokens.json",
            "encode/minimal_text_request.json",
            FinishReason::Length,
            Some((Some(3), Some(3), Some(6), None)),
            Some("truncated"),
            None,
            None,
        ),
        (
            "decode/finish_reason_tool_use.json",
            "encode/minimal_text_request.json",
            FinishReason::ToolCalls,
            Some((Some(8), Some(4), Some(12), None)),
            None,
            Some(("call_weather_1", "lookup_weather")),
            None,
        ),
        (
            "decode/finish_reason_unknown.json",
            "encode/minimal_text_request.json",
            FinishReason::Other,
            Some((Some(2), Some(2), Some(4), None)),
            Some("other"),
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
        let adapter = anthropic_adapter(server.url());
        let request = request_fixture(request_fixture_path);

        let response = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect("run should succeed");

        assert_eq!(response.provider, ProviderId::Anthropic);
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
            assert_warning_codes(
                &response,
                &[
                    "unknown_stop_reason",
                    "unknown_content_block_mapped_to_text",
                ],
            );
        }

        server.shutdown();
    }
}

#[test]
fn test_anthropic_fixture_category_matrix_coverage() {
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
                "encode/tools_choice_required.json",
                "encode/tools_choice_specific.json",
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
                "decode/finish_reason_end_turn.json",
                "decode/finish_reason_max_tokens.json",
                "decode/finish_reason_tool_use.json",
                "decode/finish_reason_unknown.json",
            ],
        ),
    ];

    let error_edge_categories: &[(&str, &[&str])] = &[
        (
            "protocol error payload mapping",
            &["errors/error_envelope_protocol_mapping.json"],
        ),
        (
            "unsupported canonical intent",
            &["errors/unsupported_intent_json_prefill.json"],
        ),
        (
            "malformed payload decode failures",
            &[
                "errors/malformed_payload_non_object.json",
                "errors/malformed_payload_missing_stop_reason.json",
                "errors/malformed_payload_tool_use_non_object_input.json",
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
                "errors/malformed_payload_missing_stop_reason.json",
                "errors/unsupported_intent_json_prefill.json",
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
async fn test_anthropic_contract_non_2xx_auth_maps_to_credentials_rejected() {
    let error_body = load_fixture_str("errors/error_envelope_protocol_mapping.json");
    let mut server = MockServer::start(vec![MockResponse::with_status(
        401,
        vec![("request-id".to_string(), "req-contract-auth".to_string())],
        &error_body,
    )]);
    let adapter = anthropic_adapter(server.url());

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
            assert_eq!(provider, ProviderId::Anthropic);
            assert_eq!(request_id, Some("req-contract-auth".to_string()));
            assert!(message.contains("anthropic error"));
            assert!(message.contains("invalid_request_error"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_contract_non_2xx_non_auth_maps_to_status() {
    let error_body = load_fixture_str("errors/error_envelope_protocol_mapping.json");
    let response = MockResponse::with_status(
        429,
        vec![("request-id".to_string(), "req-contract-rate".to_string())],
        &error_body,
    );
    let mut server = MockServer::start(vec![response.clone(), response.clone(), response]);
    let adapter = anthropic_adapter(server.url());

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
            assert_eq!(provider, ProviderId::Anthropic);
            assert_eq!(model, Some("claude-sonnet-4-5-20250929".to_string()));
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-contract-rate".to_string()));
            assert!(message.contains("Invalid API key"));
            assert!(message.contains("invalid_request_error"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_contract_discovery_models_mapping() {
    let mut server = MockServer::start(vec![MockResponse::json(
        r#"{
            "data":[
                {"id":"claude-sonnet-4-5-20250929","display_name":"Claude Sonnet 4.5"},
                {"id":"claude-haiku-4-5-20251001"},
                {"id":"claude-sonnet-4-5-20250929"}
            ]
        }"#,
    )]);
    let adapter = anthropic_adapter(server.url());

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
    assert_eq!(models[0].model_id, "claude-sonnet-4-5-20250929");
    assert_eq!(models[1].model_id, "claude-haiku-4-5-20251001");
    assert!(
        models
            .iter()
            .all(|model| model.provider == ProviderId::Anthropic)
    );
    assert_eq!(
        models[0].display_name,
        Some("Claude Sonnet 4.5".to_string())
    );

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_contract_malformed_payload_decode_failures_are_protocol() {
    let malformed_fixtures = vec![
        "errors/malformed_payload_non_object.json",
        "errors/malformed_payload_missing_stop_reason.json",
        "errors/malformed_payload_tool_use_non_object_input.json",
    ];

    let parsed_non_object = load_fixture_json("errors/malformed_payload_non_object.json");
    assert!(parsed_non_object.is_string());

    for fixture in malformed_fixtures {
        let response_body = load_fixture_str(fixture);
        let mut server = MockServer::start(vec![MockResponse::json(&response_body)]);
        let adapter = anthropic_adapter(server.url());

        let request = request_fixture("encode/minimal_text_request.json");
        let err = adapter
            .run(&request, &AdapterContext::default())
            .await
            .expect_err("malformed payload should fail");

        match err {
            ProviderError::Protocol { provider, .. } => {
                assert_eq!(provider, ProviderId::Anthropic, "fixture {fixture}");
            }
            other => panic!("expected protocol error for {fixture}, got {other:?}"),
        }

        server.shutdown();
    }
}

#[tokio::test]
async fn test_anthropic_contract_unsupported_json_prefill_is_deterministic() {
    let request = request_fixture("errors/unsupported_intent_json_prefill.json");

    let adapter =
        AnthropicAdapter::with_base_url(Some("test-key".to_string()), "http://127.0.0.1:1")
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
        assert_error_fingerprint(&second)
    );
}

#[tokio::test]
async fn test_anthropic_contract_encode_is_deterministic_for_identical_input() {
    let request = request_fixture("determinism/determinism_encode_input.json");
    let decode_payload = load_fixture_str("decode/text_only_completed.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&decode_payload),
        MockResponse::json(&decode_payload),
    ]);
    let adapter = anthropic_adapter(server.url());

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
async fn test_anthropic_contract_decode_is_deterministic_for_identical_payload() {
    let request = request_fixture("encode/response_format_json_object.json");
    let payload = load_fixture_str("determinism/determinism_decode_payload.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&payload),
        MockResponse::json(&payload),
    ]);
    let adapter = anthropic_adapter(server.url());

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
async fn test_anthropic_contract_malformed_failure_is_deterministic() {
    let request = request_fixture("encode/minimal_text_request.json");
    let malformed = load_fixture_str("errors/malformed_payload_missing_stop_reason.json");

    let mut server = MockServer::start(vec![
        MockResponse::json(&malformed),
        MockResponse::json(&malformed),
    ]);
    let adapter = anthropic_adapter(server.url());

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
        401 => "Unauthorized",
        403 => "Forbidden",
        429 => "Too Many Requests",
        _ => "Unknown",
    }
}
