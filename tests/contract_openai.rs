use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use provider_runtime::core::error::ProviderError;
use provider_runtime::core::traits::ProviderAdapter;
use provider_runtime::core::types::{
    AdapterContext, ContentPart, DiscoveryOptions, FinishReason, Message, MessageRole, ModelRef,
    ProviderId, ProviderRequest, ResponseFormat, ToolChoice, ToolDefinition,
};
use provider_runtime::providers::openai::OpenAiAdapter;
use serde_json::json;

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

    fn captured_request_bodies(&self) -> Vec<serde_json::Value> {
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

fn request_for_contract() -> ProviderRequest {
    let mut metadata = BTreeMap::new();
    metadata.insert("trace_id".to_string(), "fixture-1".to_string());

    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(ProviderId::Openai),
            model_id: "gpt-5-mini".to_string(),
        },
        messages: vec![
            Message {
                role: MessageRole::System,
                content: vec![ContentPart::Text {
                    text: "Always return JSON.".to_string(),
                }],
            },
            Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text {
                    text: "Provide JSON with city weather".to_string(),
                }],
            },
        ],
        tools: vec![ToolDefinition {
            name: "lookup_weather".to_string(),
            description: Some("Lookup weather".to_string()),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"],
                "additionalProperties": false
            }),
        }],
        tool_choice: ToolChoice::Specific {
            name: "lookup_weather".to_string(),
        },
        response_format: ResponseFormat::JsonObject,
        temperature: Some(0.3),
        top_p: None,
        max_output_tokens: Some(64),
        stop: Vec::new(),
        metadata,
    }
}

#[tokio::test]
async fn test_openai_encode_fixture_contract() {
    let mut server = MockServer::start(vec![MockResponse::json(
        r#"{
            "status":"completed",
            "model":"gpt-5-mini",
            "output":[
                {
                    "type":"message",
                    "role":"assistant",
                    "content":[{"type":"output_text","text":"{\"ok\":true}"}]
                }
            ],
            "usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12}
        }"#,
    )]);

    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let _response = adapter
        .run(&request_for_contract(), &AdapterContext::default())
        .await
        .expect("run should succeed");

    server.shutdown();
    let bodies = server.captured_request_bodies();
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];

    assert_eq!(body.pointer("/model"), Some(&json!("gpt-5-mini")));
    assert_eq!(body.pointer("/store"), Some(&json!(false)));
    assert_eq!(
        body.pointer("/text/format/type"),
        Some(&json!("json_object"))
    );
    assert_eq!(body.pointer("/tool_choice/type"), Some(&json!("function")));
    assert_eq!(
        body.pointer("/tool_choice/name"),
        Some(&json!("lookup_weather"))
    );
    assert_eq!(body.pointer("/max_output_tokens"), Some(&json!(64)));

    let input = body
        .get("input")
        .and_then(|value| value.as_array())
        .expect("input should be array");
    assert_eq!(input.len(), 2);
    assert_eq!(input[0].pointer("/role"), Some(&json!("system")));
    assert_eq!(input[1].pointer("/role"), Some(&json!("user")));
}

#[tokio::test]
async fn test_openai_decode_fixture_contract() {
    let mut server = MockServer::start(vec![MockResponse::json(
        r#"{
            "status":"incomplete",
            "incomplete_details":{"reason":"max_output_tokens"},
            "model":"gpt-5-mini",
            "output":[
                {
                    "type":"function_call",
                    "call_id":"call_1",
                    "name":"lookup_weather",
                    "arguments":"{\"city\":\"SF\"}"
                }
            ],
            "usage":{
                "input_tokens":10,
                "output_tokens":12,
                "total_tokens":22,
                "input_tokens_details":{"cached_tokens":2}
            }
        }"#,
    )]);

    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let response = adapter
        .run(&request_for_contract(), &AdapterContext::default())
        .await
        .expect("run should succeed");

    assert_eq!(response.finish_reason, FinishReason::Length);
    assert_eq!(response.usage.input_tokens, Some(10));
    assert_eq!(response.usage.output_tokens, Some(12));
    assert_eq!(response.usage.total_tokens, Some(22));
    assert_eq!(response.usage.cached_input_tokens, Some(2));
    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning.code == "openai_incomplete_max_output_tokens")
    );
    assert!(matches!(
        response.output.content.first(),
        Some(ContentPart::ToolCall { tool_call }) if tool_call.id == "call_1"
    ));

    server.shutdown();
}

#[tokio::test]
async fn test_openai_fixture_category_matrix_coverage() {
    let cases = vec![
        (
            r#"{
                "status":"completed",
                "model":"gpt-5-mini",
                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],
                "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
            }"#,
            FinishReason::Stop,
        ),
        (
            r#"{
                "status":"incomplete",
                "incomplete_details":{"reason":"content_filter"},
                "model":"gpt-5-mini",
                "output":[{"type":"message","role":"assistant","content":[{"type":"refusal","text":"blocked"}]}],
                "usage":null
            }"#,
            FinishReason::ContentFilter,
        ),
    ];

    for (payload, expected_finish_reason) in cases {
        let mut server = MockServer::start(vec![MockResponse::json(payload)]);
        let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
            .expect("create adapter");

        let response = adapter
            .run(&request_for_contract(), &AdapterContext::default())
            .await
            .expect("run should succeed");

        assert_eq!(response.finish_reason, expected_finish_reason);
        server.shutdown();
    }
}

#[tokio::test]
async fn test_openai_contract_non_2xx_auth_maps_to_credentials_rejected() {
    let mut server = MockServer::start(vec![MockResponse::with_status(
        401,
        vec![("x-request-id".to_string(), "req-contract-auth".to_string())],
        r#"{
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        }"#,
    )]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let err = adapter
        .run(&request_for_contract(), &AdapterContext::default())
        .await
        .expect_err("auth error should fail");

    match err {
        ProviderError::CredentialsRejected {
            provider,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openai);
            assert_eq!(request_id, Some("req-contract-auth".to_string()));
            assert!(message.contains("openai error"));
            assert!(message.contains("invalid_api_key"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openai_contract_discovery_models_mapping() {
    let mut server = MockServer::start(vec![MockResponse::json(
        r#"{
            "object":"list",
            "data":[
                {"id":"gpt-5-mini","object":"model"},
                {"id":"gpt-4.1","object":"model"},
                {"id":"gpt-5-mini","object":"model"}
            ]
        }"#,
    )]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

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
    assert_eq!(models[0].model_id, "gpt-5-mini");
    assert_eq!(models[1].model_id, "gpt-4.1");
    assert!(
        models
            .iter()
            .all(|model| model.provider == ProviderId::Openai)
    );
    assert!(models.iter().all(|model| model.display_name.is_none()));
    server.shutdown();
}

#[tokio::test]
async fn test_openai_contract_non_2xx_non_auth_maps_to_status() {
    let response = MockResponse::with_status(
        429,
        vec![("x-request-id".to_string(), "req-contract-rate".to_string())],
        r#"{
            "error": {
                "message": "Rate limit exceeded",
                "type": "rate_limit_error",
                "param": "model",
                "code": "rate_limit_exceeded"
            }
        }"#,
    );
    let mut server = MockServer::start(vec![response.clone(), response.clone(), response]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let err = adapter
        .run(&request_for_contract(), &AdapterContext::default())
        .await
        .expect_err("rate limit should fail");

    match err {
        ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openai);
            assert_eq!(model, Some("gpt-5-mini".to_string()));
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-contract-rate".to_string()));
            assert!(message.contains("Rate limit exceeded"));
            assert!(message.contains("rate_limit_exceeded"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

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
