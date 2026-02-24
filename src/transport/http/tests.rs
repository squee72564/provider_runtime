use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::core::error::ProviderError;
use crate::core::types::{AdapterContext, ProviderId};
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct OkResponse {
    ok: bool,
}

#[tokio::test]
async fn test_http_transport_maps_status_errors() {
    let mut server = MockServer::start(vec![MockResponse::new(
        429,
        vec![("x-request-id".to_string(), "req-123".to_string())],
        r#"{"error":"rate limit"}"#,
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

    let ctx = AdapterContext::default();
    let result = transport
        .get_json::<OkResponse>(
            ProviderId::Openai,
            Some("gpt-4.1"),
            &format!("{}/status", server.url()),
            &ctx,
        )
        .await;

    match result {
        Err(ProviderError::Status {
            provider,
            model,
            status_code,
            request_id,
            ..
        }) => {
            assert_eq!(provider, ProviderId::Openai);
            assert_eq!(model, Some("gpt-4.1".to_string()));
            assert_eq!(status_code, 429);
            assert_eq!(request_id, Some("req-123".to_string()));
        }
        other => panic!("expected ProviderError::Status, got {other:?}"),
    }

    server.shutdown();
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn test_retry_policy_respects_max_attempts() {
    let max_attempts = 3;
    let responses = (0..max_attempts)
        .map(|_| MockResponse::new(429, vec![], r#"{"error":"retry"}"#))
        .collect::<Vec<_>>();
    let mut server = MockServer::start(responses);

    let transport = HttpTransport::new(
        1_000,
        RetryPolicy {
            max_attempts,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            retryable_status_codes: vec![429],
        },
    )
    .expect("create transport");

    let ctx = AdapterContext::default();
    let result = transport
        .get_json::<OkResponse>(
            ProviderId::Openrouter,
            Some("openrouter/test"),
            &format!("{}/retry", server.url()),
            &ctx,
        )
        .await;

    assert!(matches!(
        result,
        Err(ProviderError::Status {
            status_code: 429,
            ..
        })
    ));

    server.shutdown();
    assert_eq!(server.request_count(), max_attempts as usize);
}

#[tokio::test]
async fn test_http_transport_injects_auth_and_custom_headers() {
    let mut server = MockServer::start(vec![MockResponse::new(200, vec![], r#"{"ok":true}"#)]);

    let transport = HttpTransport::new(1_000, RetryPolicy::default()).expect("create transport");

    let mut metadata = BTreeMap::new();
    metadata.insert(
        "transport.auth.bearer_token".to_string(),
        "token-abc".to_string(),
    );
    metadata.insert(
        "transport.header.x-custom-header".to_string(),
        "custom-value".to_string(),
    );
    let ctx = AdapterContext { metadata };

    let result = transport
        .post_json::<serde_json::Value, OkResponse>(
            ProviderId::Anthropic,
            Some("claude-3.7"),
            &format!("{}/headers", server.url()),
            &serde_json::json!({"ping": true}),
            &ctx,
        )
        .await
        .expect("successful response");

    assert_eq!(result, OkResponse { ok: true });

    server.shutdown();
    let captured = server.captured_headers();
    assert_eq!(captured.len(), 1);
    let first = &captured[0];
    assert_eq!(
        first.get("authorization"),
        Some(&"Bearer token-abc".to_string())
    );
    assert_eq!(
        first.get("x-custom-header"),
        Some(&"custom-value".to_string())
    );
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
        408 => "Request Timeout",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}
