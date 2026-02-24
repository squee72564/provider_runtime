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
use crate::providers::openrouter::{OpenRouterAdapter, OpenRouterAdapterOptions};
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
fn test_openrouter_adapter_capabilities() {
    let adapter = OpenRouterAdapter::new(Some("test-key".to_string())).expect("create adapter");

    let capabilities = adapter.capabilities();
    assert!(capabilities.supports_tools);
    assert!(capabilities.supports_structured_output);
    assert!(capabilities.supports_thinking);
    assert!(capabilities.supports_remote_discovery);
}

#[tokio::test]
async fn test_openrouter_adapter_missing_key_error() {
    let adapter = OpenRouterAdapter::with_base_url(None, "http://127.0.0.1:1").expect("adapter");

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
            assert_eq!(provider, ProviderId::Openrouter);
            assert_eq!(model, Some("openai/gpt-4o-mini".to_string()));
            assert!(message.contains("missing OpenRouter API key"));
            assert!(message.contains("openrouter.api_key"));
            assert!(message.contains("OPENROUTER_API_KEY"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openrouter_adapter_uses_translator_boundary() {
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

    let adapter = OpenRouterAdapter::with_transport(
        Some("test-key".to_string()),
        "http://127.0.0.1:1",
        OpenRouterAdapterOptions::default(),
        transport,
    );

    let mut req = base_request();
    req.stop = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];

    let err = adapter
        .run(&req, &AdapterContext::default())
        .await
        .expect_err("translator should fail before transport");

    match err {
        ProviderError::Protocol { message, .. } => {
            assert!(message.contains("at most 4"));
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn test_openrouter_adapter_sets_auth_and_attribution_headers() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "id":"chatcmpl_1",
            "object":"chat.completion",
            "created":123,
            "model":"openai/gpt-4o-mini",
            "choices":[{
                "index":0,
                "finish_reason":"stop",
                "message":{"role":"assistant","content":"ok"}
            }],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}
        }"#,
    )]);

    let options = OpenRouterAdapterOptions {
        http_referer: Some("https://example.com".to_string()),
        x_title: Some("provider-runtime-tests".to_string()),
        ..Default::default()
    };

    let adapter = OpenRouterAdapter::with_base_url_and_options(
        Some("test-key".to_string()),
        server.url(),
        options,
    )
    .expect("adapter");

    let response = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect("run should succeed");
    assert_eq!(response.provider, ProviderId::Openrouter);

    server.shutdown();
    assert_eq!(server.request_count(), 1);
    let headers = server.captured_headers();
    assert_eq!(
        headers[0].get("authorization"),
        Some(&"Bearer test-key".to_string())
    );
    assert_eq!(
        headers[0].get("http-referer"),
        Some(&"https://example.com".to_string())
    );
    assert_eq!(
        headers[0].get("x-title"),
        Some(&"provider-runtime-tests".to_string())
    );
}

#[tokio::test]
async fn test_openrouter_adapter_maps_auth_status_to_credentials_rejected() {
    let mut server = MockServer::start(vec![MockResponse::new(
        401,
        vec![("x-request-id".to_string(), "req-auth-1".to_string())],
        r#"{"error":{"message":"No cookie auth credentials found","code":401}}"#,
    )]);
    let adapter = OpenRouterAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

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
            assert_eq!(provider, ProviderId::Openrouter);
            assert_eq!(request_id, Some("req-auth-1".to_string()));
            assert!(message.contains("openrouter error"));
            assert!(message.contains("code=401"));
        }
        other => panic!("expected credentials rejected error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_adapter_maps_non_auth_status_to_status_error() {
    let mut server = MockServer::start(vec![MockResponse::new(
        429,
        vec![("x-request-id".to_string(), "req-rate-1".to_string())],
        r#"{"error":{"message":"Rate limit exceeded","code":429}}"#,
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

    let adapter = OpenRouterAdapter::with_transport(
        Some("test-key".to_string()),
        server.url(),
        OpenRouterAdapterOptions::default(),
        transport,
    );

    let err = adapter
        .run(&base_request(), &AdapterContext::default())
        .await
        .expect_err("run should fail");

    match err {
        ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            message,
        } => {
            assert_eq!(provider, ProviderId::Openrouter);
            assert_eq!(model, Some("openai/gpt-4o-mini".to_string()));
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-rate-1".to_string()));
            assert!(message.contains("Rate limit exceeded"));
            assert!(message.contains("openrouter error"));
        }
        other => panic!("expected status error, got {other:?}"),
    }

    server.shutdown();
}

#[tokio::test]
async fn test_openrouter_adapter_discover_models_success_without_api_key() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{
            "data": [
                {
                    "id":"openai/gpt-4o-mini",
                    "name":"GPT-4o mini",
                    "context_length":128000,
                    "top_provider":{"context_length":128000,"max_completion_tokens":4096},
                    "supported_parameters":["temperature","tools","response_format"]
                },
                {
                    "id":"openai/gpt-4o-mini",
                    "name":"GPT-4o mini dup",
                    "supported_parameters":["temperature"]
                }
            ]
        }"#,
    )]);

    let adapter = OpenRouterAdapter::with_base_url(None, server.url()).expect("adapter");

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

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "openai/gpt-4o-mini");
    assert!(models[0].supports_tools);
    assert!(models[0].supports_structured_output);

    server.shutdown();
    let headers = server.captured_headers();
    assert_eq!(headers[0].get("authorization"), None);
}

#[tokio::test]
async fn test_openrouter_adapter_discover_models_with_api_key_adds_auth_header() {
    let mut server = MockServer::start(vec![MockResponse::new(
        200,
        vec![],
        r#"{"data":[{"id":"openai/gpt-4o-mini","name":"GPT-4o mini","supported_parameters":[]}]}"#,
    )]);

    let adapter = OpenRouterAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("adapter");

    let _models = adapter
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

    server.shutdown();
    let headers = server.captured_headers();
    assert_eq!(
        headers[0].get("authorization"),
        Some(&"Bearer test-key".to_string())
    );
}

#[tokio::test]
async fn test_openrouter_adapter_discover_models_invalid_payload_is_protocol_error() {
    let mut server =
        MockServer::start(vec![MockResponse::new(200, vec![], r#"{"object":"list"}"#)]);

    let adapter = OpenRouterAdapter::with_base_url(None, server.url()).expect("adapter");

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
    server.shutdown();
}

#[test]
fn test_openrouter_options_validation() {
    let bad = OpenRouterAdapter::with_base_url_and_options(
        None,
        "http://example.com",
        OpenRouterAdapterOptions {
            fallback_models: vec![" ".to_string()],
            ..Default::default()
        },
    );
    let bad = match bad {
        Ok(_) => panic!("empty fallback should fail"),
        Err(error) => error,
    };
    assert!(bad.to_string().contains("fallback_models"));

    let bad_provider = OpenRouterAdapter::with_base_url_and_options(
        None,
        "http://example.com",
        OpenRouterAdapterOptions {
            provider_preferences: Some(json::from_str("[]").expect("json array")),
            ..Default::default()
        },
    );
    let bad_provider = match bad_provider {
        Ok(_) => panic!("provider preferences array should fail"),
        Err(error) => error,
    };
    assert!(bad_provider.to_string().contains("provider_preferences"));
}

mod json {
    pub fn from_str(input: &str) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(input)
    }
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
