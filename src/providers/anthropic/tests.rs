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
use crate::providers::anthropic::AnthropicAdapter;
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
        max_output_tokens: Some(16),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn test_anthropic_adapter_capabilities() {
    let adapter = AnthropicAdapter::new(Some("test-key".to_string())).expect("create adapter");

    let capabilities = adapter.capabilities();
    assert!(capabilities.supports_tools);
    assert!(capabilities.supports_structured_output);
    assert!(capabilities.supports_thinking);
    assert!(capabilities.supports_remote_discovery);
}

#[tokio::test]
async fn test_anthropic_adapter_missing_key_error() {
    let adapter = AnthropicAdapter::with_base_url(None, "http://127.0.0.1:1").expect("adapter");

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("run should fail");

    match err {
        ProviderError::Protocol {
            provider,
            model,
            message,
            ..
        } => {
            assert_eq!(provider, ProviderId::Anthropic);
            assert_eq!(model, Some("claude-sonnet-4-5".to_string()));
            assert!(message.contains("missing Anthropic API key"));
            assert!(message.contains("anthropic.api_key"));
            assert!(message.contains("ANTHROPIC_API_KEY"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_anthropic_adapter_uses_translator_boundary() {
    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("transport");

    let adapter = AnthropicAdapter::with_transport(
        Some("test-key".to_string()),
        "http://127.0.0.1:1",
        transport,
    );

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

    let err = adapter
        .run(&req, &AdapterContext::default())
        .await
        .expect_err("translator should fail before transport");

    match err {
        ProviderError::Protocol { message, .. } => {
            assert!(message.contains("system messages must form a contiguous prefix"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_anthropic_adapter_sets_required_headers() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "id":"msg_1",
            "type":"message",
            "role":"assistant",
            "model":"claude-sonnet-4-5",
            "stop_reason":"end_turn",
            "content":[{"type":"text","text":"ok"}],
            "usage":{"input_tokens":1,"output_tokens":1}
        }"#,
    )]);

    let adapter = AnthropicAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

    let response = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect("run should succeed");

    assert_eq!(response.provider, ProviderId::Anthropic);

    server.shutdown();
    assert_eq!(server.request_count(), 1);
    let headers = server.captured_headers();
    assert_eq!(headers[0].get("x-api-key"), Some(&"test-key".to_string()));
    assert_eq!(
        headers[0].get("anthropic-version"),
        Some(&"2023-06-01".to_string())
    );
    assert_eq!(headers[0].get("authorization"), None);
}

#[tokio::test]
async fn test_anthropic_adapter_maps_auth_status_to_credentials_rejected() {
    let mut server = MockServer::start(vec![MockResponse::new(
        401,
        vec![("request-id".to_string(), "req-auth-1".to_string())],
        r#"{
            "type":"error",
            "error":{"type":"authentication_error","message":"invalid x-api-key"},
            "request_id":"req-auth-body"
        }"#,
    )]);

    let adapter = AnthropicAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("auth error expected");

    match err {
        ProviderError::CredentialsRejected {
            provider,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Anthropic);
            assert_eq!(request_id, Some("req-auth-1".to_string()));
            assert!(message.contains("anthropic error"));
            assert!(message.contains("authentication_error"));
        }
        other => panic!("expected credentials rejected, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_adapter_maps_non_auth_status_to_status_error() {
    let mut server = MockServer::start(vec![MockResponse::new(
        529,
        vec![("request-id".to_string(), "req-overloaded-1".to_string())],
        r#"{
            "type":"error",
            "error":{"type":"overloaded_error","message":"overloaded"},
            "request_id":"req-overloaded-body"
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
    .expect("transport");

    let adapter =
        AnthropicAdapter::with_transport(Some("test-key".to_string()), server.url(), transport);

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("status error expected");

    match err {
        ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Anthropic);
            assert_eq!(model, Some("claude-sonnet-4-5".to_string()));
            assert_eq!(status_code, 529);
            assert_eq!(request_id, Some("req-overloaded-1".to_string()));
            assert!(message.contains("anthropic error"));
            assert!(message.contains("overloaded_error"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_adapter_discover_models_success() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "data":[
                {"id":"claude-sonnet-4-5","display_name":"Claude Sonnet 4.5"},
                {"id":"claude-haiku-4-5"},
                {"id":"claude-sonnet-4-5"}
            ]
        }"#,
    )]);

    let adapter = AnthropicAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

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
    assert_eq!(models[0].model_id, "claude-sonnet-4-5");
    assert_eq!(models[1].model_id, "claude-haiku-4-5");

    server.shutdown();
}

#[tokio::test]
async fn test_anthropic_adapter_discover_models_invalid_payload_is_protocol_error() {
    let mut server =
        MockServer::start(vec![MockResponse::new(200, vec![], r#"{"object":"list"}"#)]);

    let adapter = AnthropicAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

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

    assert!(matches!(err, ProviderError::Protocol { .. }));

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
        529 => "Overloaded",
        _ => "Unknown",
    }
}
