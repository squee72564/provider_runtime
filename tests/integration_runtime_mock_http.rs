use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use provider_runtime::ProviderRuntime;
use provider_runtime::core::types::{
    ContentPart, FinishReason, Message, MessageRole, ModelCatalog, ModelInfo, ModelRef, ProviderId,
    ProviderRequest, ResponseFormat, ToolCall, ToolChoice, ToolDefinition,
};
use provider_runtime::pricing::{PriceRule, PricingTable};
use provider_runtime::providers::anthropic::AnthropicAdapter;
use provider_runtime::providers::openai::OpenAiAdapter;
use provider_runtime::providers::openrouter::OpenRouterAdapter;
use serde_json::json;

const FIXTURE_ROOT: &str = "tests/fixtures/integration/runtime_mock_http";

#[derive(Debug, Clone)]
struct MockResponse {
    status_code: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl MockResponse {
    fn json(body: String) -> Self {
        Self {
            status_code: 200,
            headers: Vec::new(),
            body,
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

    fn captured_request_paths(&self) -> Vec<String> {
        self.captured_requests
            .lock()
            .expect("capture lock")
            .iter()
            .map(|raw_request| {
                let request_line = raw_request.lines().next().unwrap_or_default();
                request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string()
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

fn request_for(
    provider_hint: Option<ProviderId>,
    model_id: &str,
    with_tool: bool,
) -> ProviderRequest {
    let tools = if with_tool {
        vec![ToolDefinition {
            name: "calculator".to_string(),
            description: Some("simple calculator".to_string()),
            parameters_schema: json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string" }
                },
                "required": ["expression"]
            }),
        }]
    } else {
        Vec::new()
    };

    ProviderRequest {
        model: ModelRef {
            provider_hint,
            model_id: model_id.to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "What is the weather today?".to_string(),
            }],
        }],
        tools,
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: None,
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn runtime_with_single_adapter(
    adapter: Arc<dyn provider_runtime::core::traits::ProviderAdapter>,
) -> ProviderRuntime {
    ProviderRuntime::builder().with_adapter(adapter).build()
}

fn runtime_multi_provider(
    adapters: Vec<Arc<dyn provider_runtime::core::traits::ProviderAdapter>>,
    catalog: ModelCatalog,
    default_provider: ProviderId,
    pricing_table: Option<PricingTable>,
) -> ProviderRuntime {
    let mut builder = ProviderRuntime::builder()
        .with_model_catalog(catalog)
        .with_default_provider(default_provider);

    for adapter in adapters {
        builder = builder.with_adapter(adapter);
    }

    if let Some(table) = pricing_table {
        builder = builder.with_pricing_table(table);
    }

    builder.build()
}

fn assert_has_text_content(content: &[ContentPart]) {
    assert!(
        content
            .iter()
            .any(|part| matches!(part, ContentPart::Text { text } if !text.trim().is_empty())),
        "expected at least one non-empty text part"
    );
}

fn assert_has_tool_call(content: &[ContentPart]) {
    assert!(
        content.iter().any(|part| matches!(
            part,
            ContentPart::ToolCall {
                tool_call: ToolCall { id, name, .. }
            } if !id.is_empty() && !name.is_empty()
        )),
        "expected at least one tool call part"
    );
}

fn assert_has_warning_code(response: &provider_runtime::core::types::ProviderResponse, code: &str) {
    assert!(
        response.warnings.iter().any(|warning| warning.code == code),
        "expected warning code `{code}`, got {:?}",
        response
            .warnings
            .iter()
            .map(|warning| warning.code.as_str())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn test_runtime_run_openai_mock() {
    let basic = load_fixture_str("openai/basic_chat_gpt-5.2.json");
    let tool = load_fixture_str("openai/tool_call_gpt-5.2.json");

    let mut server = MockServer::start(vec![MockResponse::json(basic), MockResponse::json(tool)]);
    let adapter = OpenAiAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create openai adapter");

    let runtime = runtime_with_single_adapter(Arc::new(adapter));

    let basic_response = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", false))
        .await
        .expect("openai basic run should succeed");
    let tool_response = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", true))
        .await
        .expect("openai tool run should succeed");

    assert_eq!(basic_response.provider, ProviderId::Openai);
    assert_eq!(tool_response.provider, ProviderId::Openai);
    assert_eq!(basic_response.finish_reason, FinishReason::Stop);
    assert_eq!(tool_response.finish_reason, FinishReason::ToolCalls);
    assert_has_text_content(&basic_response.output.content);
    assert_has_tool_call(&tool_response.output.content);
    assert!(basic_response.usage.input_tokens.is_some());
    assert!(basic_response.usage.output_tokens.is_some());
    assert!(tool_response.usage.input_tokens.is_some());
    assert!(tool_response.usage.output_tokens.is_some());

    server.shutdown();
    assert_eq!(
        server.captured_request_paths(),
        vec!["/v1/responses".to_string(), "/v1/responses".to_string()]
    );
}

#[tokio::test]
async fn test_runtime_run_anthropic_mock() {
    let basic = load_fixture_str("anthropic/basic_chat_claude-opus-4-6.json");
    let tool = load_fixture_str("anthropic/tool_call_claude-opus-4-5-20251101.json");

    let mut server = MockServer::start(vec![MockResponse::json(basic), MockResponse::json(tool)]);
    let adapter = AnthropicAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create anthropic adapter");

    let runtime = runtime_with_single_adapter(Arc::new(adapter));

    let basic_response = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-6",
            false,
        ))
        .await
        .expect("anthropic basic run should succeed");
    let tool_response = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-5-20251101",
            true,
        ))
        .await
        .expect("anthropic tool run should succeed");

    assert_eq!(basic_response.provider, ProviderId::Anthropic);
    assert_eq!(tool_response.provider, ProviderId::Anthropic);
    assert_eq!(basic_response.finish_reason, FinishReason::Stop);
    assert_eq!(tool_response.finish_reason, FinishReason::ToolCalls);
    assert_has_text_content(&basic_response.output.content);
    assert_has_tool_call(&tool_response.output.content);
    assert!(basic_response.usage.input_tokens.is_some());
    assert!(basic_response.usage.output_tokens.is_some());
    assert!(tool_response.usage.input_tokens.is_some());
    assert!(tool_response.usage.output_tokens.is_some());

    server.shutdown();
    assert_eq!(
        server.captured_request_paths(),
        vec!["/v1/messages".to_string(), "/v1/messages".to_string()]
    );
}

#[tokio::test]
async fn test_runtime_run_openrouter_mock() {
    let basic = load_fixture_str("openrouter/basic_chat_openai.gpt-5.2.json");
    let tool = load_fixture_str("openrouter/tool_call_openai.gpt-5.2.json");

    let mut server = MockServer::start(vec![MockResponse::json(basic), MockResponse::json(tool)]);
    let adapter = OpenRouterAdapter::with_base_url(Some("test-key".to_string()), server.url())
        .expect("create openrouter adapter");

    let runtime = runtime_with_single_adapter(Arc::new(adapter));

    let basic_response = runtime
        .run(request_for(
            Some(ProviderId::Openrouter),
            "openai/gpt-5.2",
            false,
        ))
        .await
        .expect("openrouter basic run should succeed");
    let tool_response = runtime
        .run(request_for(
            Some(ProviderId::Openrouter),
            "openai/gpt-5.2",
            true,
        ))
        .await
        .expect("openrouter tool run should succeed");

    assert_eq!(basic_response.provider, ProviderId::Openrouter);
    assert_eq!(tool_response.provider, ProviderId::Openrouter);
    assert_eq!(basic_response.finish_reason, FinishReason::Stop);
    assert_eq!(tool_response.finish_reason, FinishReason::ToolCalls);
    assert_has_text_content(&basic_response.output.content);
    assert_has_tool_call(&tool_response.output.content);
    assert!(basic_response.usage.input_tokens.is_some());
    assert!(basic_response.usage.output_tokens.is_some());
    assert!(tool_response.usage.input_tokens.is_some());
    assert!(tool_response.usage.output_tokens.is_some());

    server.shutdown();
    assert_eq!(
        server.captured_request_paths(),
        vec![
            "/api/v1/chat/completions".to_string(),
            "/api/v1/chat/completions".to_string()
        ]
    );
}

#[tokio::test]
async fn test_runtime_routing_and_warning_behavior() {
    let openai_payload = load_fixture_str("openai/basic_chat_gpt-5.2.json");
    let anthropic_payload = load_fixture_str("anthropic/basic_chat_claude-opus-4-6.json");
    let openrouter_payload = load_fixture_str("openrouter/basic_chat_openai.gpt-5.2.json");

    let mut openai_server = MockServer::start(vec![MockResponse::json(openai_payload)]);
    let mut anthropic_server = MockServer::start(vec![MockResponse::json(anthropic_payload)]);
    let mut openrouter_server = MockServer::start(vec![MockResponse::json(openrouter_payload)]);

    let openai_adapter =
        OpenAiAdapter::with_base_url(Some("test-key".to_string()), openai_server.url())
            .expect("create openai adapter");
    let anthropic_adapter =
        AnthropicAdapter::with_base_url(Some("test-key".to_string()), anthropic_server.url())
            .expect("create anthropic adapter");
    let openrouter_adapter =
        OpenRouterAdapter::with_base_url(Some("test-key".to_string()), openrouter_server.url())
            .expect("create openrouter adapter");

    let catalog = ModelCatalog {
        models: vec![
            ModelInfo {
                provider: ProviderId::Anthropic,
                model_id: "catalog-anthropic-model".to_string(),
                display_name: Some("Catalog Anthropic".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
            ModelInfo {
                provider: ProviderId::Openrouter,
                model_id: "openrouter/auto".to_string(),
                display_name: Some("OpenRouter Auto".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
        ],
    };

    let pricing_table = PricingTable::new(vec![PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5.2*".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
    }]);

    let runtime = runtime_multi_provider(
        vec![
            Arc::new(openai_adapter),
            Arc::new(anthropic_adapter),
            Arc::new(openrouter_adapter),
        ],
        catalog,
        ProviderId::Openrouter,
        Some(pricing_table),
    );

    let by_hint = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", false))
        .await
        .expect("hint-routed run should succeed");
    let by_catalog = runtime
        .run(request_for(None, "catalog-anthropic-model", false))
        .await
        .expect("catalog-routed run should succeed");
    let by_default = runtime
        .run(request_for(None, "missing-model-id", false))
        .await
        .expect("default-routed run should succeed");

    assert_eq!(by_hint.provider, ProviderId::Openai);
    assert!(by_hint.cost.is_some(), "openai run should get priced");

    assert_eq!(by_catalog.provider, ProviderId::Anthropic);
    assert!(
        by_catalog.cost.is_none(),
        "anthropic run should not get priced"
    );
    assert_has_warning_code(&by_catalog, "pricing.missing_rule");

    assert_eq!(by_default.provider, ProviderId::Openrouter);
    assert!(
        by_default.cost.is_none(),
        "openrouter run should not get priced"
    );
    assert_has_warning_code(&by_default, "pricing.missing_rule");

    openai_server.shutdown();
    anthropic_server.shutdown();
    openrouter_server.shutdown();

    assert_eq!(
        openai_server.captured_request_paths(),
        vec!["/v1/responses".to_string()]
    );
    assert_eq!(
        anthropic_server.captured_request_paths(),
        vec!["/v1/messages".to_string()]
    );
    assert_eq!(
        openrouter_server.captured_request_paths(),
        vec!["/api/v1/chat/completions".to_string()]
    );
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
