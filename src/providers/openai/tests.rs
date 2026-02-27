use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::core::error::ProviderError;
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, ContentPart, DiscoveryOptions, Message, MessageRole, ModelRef, ProviderId,
    ProviderRequest, ResponseFormat, ToolChoice,
};
use crate::providers::openai::OpenAiAdapter;
use crate::transport::http::{HttpTransport, RetryPolicy};

#[derive(Debug, Clone)]
struct MockResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl MockResponse {
    fn new(status_code: u16, headers: Vec<(String, String)>, body: &str) -> Self {
        Self {
            status_code,
            headers,
            body: body.to_string(),
        }
    }
}

struct MockServer {
    addr: std::net::SocketAddr,
    request_count: Arc<AtomicUsize>,
    captured_headers: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
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
        let request_count = Arc::new(AtomicUsize::new(0));
        let captured_headers = Arc::new(Mutex::new(Vec::new()));

        let queue_clone = Arc::clone(&queue);
        let request_count_clone = Arc::clone(&request_count);
        let captured_headers_clone = Arc::clone(&captured_headers);

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

                let request = read_http_request(&mut stream);
                let headers = parse_request_headers(&request);
                captured_headers_clone
                    .lock()
                    .expect("captured headers lock")
                    .push(headers);
                request_count_clone.fetch_add(1, Ordering::SeqCst);

                let response_text = build_http_response(&response);
                stream
                    .write_all(response_text.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
            }
        });

        Self {
            addr,
            request_count,
            captured_headers,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::SeqCst)
    }

    fn captured_headers(&self) -> Vec<BTreeMap<String, String>> {
        self.captured_headers
            .lock()
            .expect("captured headers lock")
            .clone()
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
fn test_openai_adapter_capabilities() {
    let adapter = OpenAiAdapter::new(Some("test-key".to_string())).expect("create adapter");

    let capabilities = adapter.capabilities();
    assert!(capabilities.supports_tools);
    assert!(capabilities.supports_structured_output);
    assert!(!capabilities.supports_thinking);
    assert!(capabilities.supports_remote_discovery);
}

#[tokio::test]
async fn test_openai_adapter_missing_key_error() {
    let adapter = OpenAiAdapter::with_base_url(None, "http://127.0.0.1:1").expect("create adapter");
    let req = base_request();

    let err = adapter
        .run(&req, &AdapterContext::default())
        .await
        .expect_err("run should fail");

    match err {
        ProviderError::Protocol {
            provider,
            model,
            message,
            ..
        } => {
            assert_eq!(provider, ProviderId::Openai);
            assert_eq!(model, Some("gpt-5-mini".to_string()));
            assert!(message.contains("missing OpenAI API key"));
            assert!(message.contains("openai.api_key"));
            assert!(message.contains("OPENAI_API_KEY"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openai_adapter_uses_translator_boundary() {
    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("create transport");

    let adapter = OpenAiAdapter::with_transport(
        Some("test-key".to_string()),
        "http://127.0.0.1:1",
        transport,
    );

    let mut req = base_request();
    req.stop.push("STOP".to_string());

    let err = adapter
        .run(&req, &AdapterContext::default())
        .await
        .expect_err("stop should fail via translator");

    match err {
        ProviderError::Protocol { message, .. } => {
            assert!(message.contains("stop sequences are unsupported"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openai_adapter_propagates_encode_warnings() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "status":"completed",
            "model":"gpt-5-mini",
            "output":[
                {
                    "type":"message",
                    "role":"assistant",
                    "content":[{"type":"output_text","text":"ok"}]
                }
            ],
            "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
        }"#,
    )]);

    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("create transport");

    let adapter =
        OpenAiAdapter::with_transport(Some("test-key".to_string()), server.url(), transport);

    let mut req = base_request();
    req.temperature = Some(0.2);
    req.top_p = Some(0.8);

    let response = adapter
        .run(&req, &AdapterContext::default())
        .await
        .expect("run should succeed");

    assert!(
        response
            .warnings
            .iter()
            .any(|warning| warning.code == "both_temperature_and_top_p_set")
    );

    server.shutdown();
    assert_eq!(server.request_count(), 1);
    let headers = server.captured_headers();
    assert_eq!(
        headers[0].get("authorization"),
        Some(&"Bearer test-key".to_string())
    );
}

#[tokio::test]
async fn test_openai_adapter_maps_auth_status_to_credentials_rejected() {
    let mut server = MockServer::start(vec![MockResponse::new(
        401,
        vec![("x-request-id".to_string(), "req-auth-1".to_string())],
        r#"{
            "error": {
                "message": "Invalid API key provided",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        }"#,
    )]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("auth failure should fail");

    match err {
        ProviderError::CredentialsRejected {
            provider,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openai);
            assert_eq!(request_id, Some("req-auth-1".to_string()));
            assert!(message.contains("openai error"));
            assert!(message.contains("invalid_api_key"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openai_adapter_maps_non_auth_status_to_normalized_status() {
    let mut server = MockServer::start(vec![MockResponse::new(
        429,
        vec![("x-request-id".to_string(), "req-rate-1".to_string())],
        r#"{
            "error": {
                "message": "Rate limit exceeded",
                "type": "rate_limit_error",
                "param": "model",
                "code": "rate_limit_exceeded"
            }
        }"#,
    )]);
    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("create transport");
    let adapter =
        OpenAiAdapter::with_transport(Some("test-key".to_string()), server.url(), transport);

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
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
            assert_eq!(request_id, Some("req-rate-1".to_string()));
            assert!(message.contains("Rate limit exceeded"));
            assert!(message.contains("type=rate_limit_error"));
            assert!(message.contains("param=model"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openai_adapter_status_fallback_when_error_body_is_not_json() {
    let mut server = MockServer::start(vec![MockResponse::new(
        429,
        vec![("x-request-id".to_string(), "req-raw-1".to_string())],
        "not-json",
    )]);
    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("create transport");
    let adapter =
        OpenAiAdapter::with_transport(Some("test-key".to_string()), server.url(), transport);

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("status should fail");

    match err {
        ProviderError::Status {
            status_code,
            request_id,
            message,
            ..
        } => {
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-raw-1".to_string()));
            assert_eq!(message, "not-json");
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openai_adapter_discover_models_success() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "object": "list",
            "data": [
                {"id": "gpt-5-mini", "object": "model"},
                {"id": "gpt-4.1", "object": "model"},
                {"id": "gpt-5-mini", "object": "model"}
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
    assert!(models.iter().all(|model| model.supports_tools));
    assert!(models.iter().all(|model| model.supports_structured_output));

    server.shutdown();
}

#[tokio::test]
async fn test_openai_adapter_discover_models_missing_key_error() {
    let adapter = OpenAiAdapter::with_base_url(None, "http://127.0.0.1:1").expect("create adapter");
    let err = adapter
        .discover_models(
            &DiscoveryOptions {
                remote: true,
                include_provider: Vec::new(),
                refresh_cache: true,
            },
            &AdapterContext::default(),
        )
        .await
        .expect_err("discover should fail without key");

    match err {
        ProviderError::Protocol { message, .. } => {
            assert!(message.contains("missing OpenAI API key"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openai_adapter_discover_models_auth_error_is_credentials_rejected() {
    let mut server = MockServer::start(vec![MockResponse::new(
        401,
        vec![("x-request-id".to_string(), "req-auth-discover".to_string())],
        r#"{
            "error": {
                "message": "Unauthorized",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        }"#,
    )]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let err = adapter
        .discover_models(
            &DiscoveryOptions {
                remote: true,
                include_provider: Vec::new(),
                refresh_cache: true,
            },
            &AdapterContext::default(),
        )
        .await
        .expect_err("discover should fail");

    match err {
        ProviderError::CredentialsRejected {
            request_id,
            message,
            ..
        } => {
            assert_eq!(request_id, Some("req-auth-discover".to_string()));
            assert!(message.contains("invalid_api_key"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openai_adapter_discover_models_invalid_payload_is_protocol_error() {
    let mut server =
        MockServer::start(vec![MockResponse::new(200, vec![], r#"{"object":"list"}"#)]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create adapter");

    let err = adapter
        .discover_models(
            &DiscoveryOptions {
                remote: true,
                include_provider: Vec::new(),
                refresh_cache: true,
            },
            &AdapterContext::default(),
        )
        .await
        .expect_err("discover should fail on invalid payload");

    assert!(matches!(err, ProviderError::Protocol { .. }));
    assert!(err.to_string().contains("missing data array"));
    server.shutdown();
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(bytes_read) => {
                request.extend_from_slice(&chunk[..bytes_read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
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

fn parse_request_headers(raw_request: &str) -> BTreeMap<String, String> {
    raw_request
        .split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect()
}

fn build_http_response(response: &MockResponse) -> String {
    let mut rendered = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status_code,
        status_reason(response.status_code),
        response.body.len(),
    );
    for (name, value) in &response.headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    rendered.push_str("\r\n");
    rendered.push_str(&response.body);
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
