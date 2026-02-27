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
    ProviderRequest, ProviderResponse, ResponseFormat, ToolChoice, ToolDefinition, ToolResult,
    ToolResultContent,
};
use provider_runtime::handoff::normalize_handoff_messages;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalProjection {
    finish_reason: FinishReason,
    text_parts: usize,
    tool_call_parts: usize,
    tool_result_parts: usize,
    has_tool_calls: bool,
    structured_output_present: bool,
    has_input_tokens: bool,
    has_output_tokens: bool,
    has_total_tokens: bool,
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

fn runtime_multi_provider(
    adapters: Vec<Arc<dyn provider_runtime::core::traits::ProviderAdapter>>,
    pricing_table: Option<PricingTable>,
) -> ProviderRuntime {
    let catalog = ModelCatalog {
        models: vec![
            ModelInfo {
                provider: ProviderId::Openai,
                model_id: "gpt-5.2".to_string(),
                display_name: Some("OpenAI GPT-5.2".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
            ModelInfo {
                provider: ProviderId::Anthropic,
                model_id: "claude-opus-4-6".to_string(),
                display_name: Some("Anthropic Claude Opus 4.6".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
            ModelInfo {
                provider: ProviderId::Openrouter,
                model_id: "openai/gpt-5.2".to_string(),
                display_name: Some("OpenRouter OpenAI GPT-5.2".to_string()),
                context_window: None,
                max_output_tokens: None,
                supports_tools: true,
                supports_structured_output: true,
            },
        ],
    };

    let mut builder = ProviderRuntime::builder()
        .with_model_catalog(catalog)
        .with_default_provider(ProviderId::Openrouter);

    for adapter in adapters {
        builder = builder.with_adapter(adapter);
    }

    if let Some(table) = pricing_table {
        builder = builder.with_pricing_table(table);
    }

    builder.build()
}

fn project_response(response: &ProviderResponse) -> CanonicalProjection {
    let mut text_parts = 0;
    let mut tool_call_parts = 0;
    let mut tool_result_parts = 0;

    for part in &response.output.content {
        match part {
            ContentPart::Text { .. } => text_parts += 1,
            ContentPart::ToolCall { .. } => tool_call_parts += 1,
            ContentPart::ToolResult { .. } => tool_result_parts += 1,
        }
    }

    CanonicalProjection {
        finish_reason: response.finish_reason.clone(),
        text_parts,
        tool_call_parts,
        tool_result_parts,
        has_tool_calls: tool_call_parts > 0,
        structured_output_present: response.output.structured_output.is_some(),
        has_input_tokens: response.usage.input_tokens.is_some(),
        has_output_tokens: response.usage.output_tokens.is_some(),
        has_total_tokens: response.usage.total_tokens.is_some(),
    }
}

fn first_tool_call_id(response: &ProviderResponse) -> Option<String> {
    response.output.content.iter().find_map(|part| {
        if let ContentPart::ToolCall { tool_call } = part {
            Some(tool_call.id.clone())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn test_multi_provider_canonical_equivalence() {
    let mut openai_server = MockServer::start(vec![
        MockResponse::json(load_fixture_str("openai/basic_chat_gpt-5.2.json")),
        MockResponse::json(load_fixture_str("openai/tool_call_gpt-5.2.json")),
    ]);
    let mut anthropic_server = MockServer::start(vec![
        MockResponse::json(load_fixture_str(
            "anthropic/basic_chat_claude-opus-4-6.json",
        )),
        MockResponse::json(load_fixture_str(
            "anthropic/tool_call_claude-opus-4-5-20251101.json",
        )),
    ]);
    let mut openrouter_server = MockServer::start(vec![
        MockResponse::json(load_fixture_str(
            "openrouter/basic_chat_openai.gpt-5.2.json",
        )),
        MockResponse::json(load_fixture_str("openrouter/tool_call_openai.gpt-5.2.json")),
    ]);

    let runtime = runtime_multi_provider(
        vec![
            Arc::new(
                OpenAiAdapter::with_base_url(Some("test-key".to_string()), openai_server.url())
                    .expect("create openai adapter"),
            ),
            Arc::new(
                AnthropicAdapter::with_base_url(
                    Some("test-key".to_string()),
                    anthropic_server.url(),
                )
                .expect("create anthropic adapter"),
            ),
            Arc::new(
                OpenRouterAdapter::with_base_url(
                    Some("test-key".to_string()),
                    openrouter_server.url(),
                )
                .expect("create openrouter adapter"),
            ),
        ],
        None,
    );

    let openai_basic = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", false))
        .await
        .expect("openai basic run should succeed");
    let anthropic_basic = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-6",
            false,
        ))
        .await
        .expect("anthropic basic run should succeed");
    let openrouter_basic = runtime
        .run(request_for(
            Some(ProviderId::Openrouter),
            "openai/gpt-5.2",
            false,
        ))
        .await
        .expect("openrouter basic run should succeed");

    let openai_tool = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", true))
        .await
        .expect("openai tool run should succeed");
    let anthropic_tool = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-6",
            true,
        ))
        .await
        .expect("anthropic tool run should succeed");
    let openrouter_tool = runtime
        .run(request_for(
            Some(ProviderId::Openrouter),
            "openai/gpt-5.2",
            true,
        ))
        .await
        .expect("openrouter tool run should succeed");

    let basic_projection = project_response(&openai_basic);
    assert_eq!(basic_projection, project_response(&anthropic_basic));
    assert_eq!(basic_projection, project_response(&openrouter_basic));
    assert_eq!(basic_projection.finish_reason, FinishReason::Stop);

    let tool_projection = project_response(&openai_tool);
    assert_eq!(tool_projection, project_response(&anthropic_tool));
    assert_eq!(tool_projection, project_response(&openrouter_tool));
    assert_eq!(tool_projection.finish_reason, FinishReason::ToolCalls);
    assert!(tool_projection.has_tool_calls);

    openai_server.shutdown();
    anthropic_server.shutdown();
    openrouter_server.shutdown();

    assert_eq!(
        openai_server.captured_request_paths(),
        vec!["/v1/responses".to_string(), "/v1/responses".to_string()]
    );
    assert_eq!(
        anthropic_server.captured_request_paths(),
        vec!["/v1/messages".to_string(), "/v1/messages".to_string()]
    );
    assert_eq!(
        openrouter_server.captured_request_paths(),
        vec![
            "/api/v1/chat/completions".to_string(),
            "/api/v1/chat/completions".to_string()
        ]
    );
}

#[tokio::test]
async fn test_handoff_deterministic_behavior() {
    let mut openai_server = MockServer::start(vec![
        MockResponse::json(load_fixture_str("openai/basic_chat_gpt-5.2.json")),
        MockResponse::json(load_fixture_str("openai/tool_call_gpt-5.2.json")),
    ]);
    let mut anthropic_server = MockServer::start(vec![MockResponse::json(load_fixture_str(
        "anthropic/tool_call_claude-opus-4-5-20251101.json",
    ))]);
    let runtime = runtime_multi_provider(
        vec![
            Arc::new(
                OpenAiAdapter::with_base_url(Some("test-key".to_string()), openai_server.url())
                    .expect("create openai adapter"),
            ),
            Arc::new(
                AnthropicAdapter::with_base_url(
                    Some("test-key".to_string()),
                    anthropic_server.url(),
                )
                .expect("create anthropic adapter"),
            ),
        ],
        None,
    );

    let openai_basic = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", false))
        .await
        .expect("openai basic run should succeed");
    let openai_tool = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", true))
        .await
        .expect("openai tool run should succeed");
    let anthropic_tool = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-6",
            true,
        ))
        .await
        .expect("anthropic tool run should succeed");

    let openai_tool_call_id = first_tool_call_id(&openai_tool).expect("expected openai tool call");

    let history = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Find the weather then summarize".to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: openai_basic.output.content.clone(),
        },
        Message {
            role: MessageRole::Assistant,
            content: anthropic_tool.output.content.clone(),
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: openai_tool_call_id,
                    content: ToolResultContent::Text {
                        text: "sunny and 68F".to_string(),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];
    let original = history.clone();

    for target in [
        ProviderId::Openai,
        ProviderId::Anthropic,
        ProviderId::Openrouter,
    ] {
        let once = normalize_handoff_messages(&history, &target);
        let twice = normalize_handoff_messages(&once, &target);
        assert_eq!(once, history, "handoff should be identity for {target:?}");
        assert_eq!(twice, once, "handoff should be idempotent for {target:?}");
    }

    assert_eq!(history, original, "source history should remain unchanged");

    openai_server.shutdown();
    anthropic_server.shutdown();
}

#[tokio::test]
async fn test_optional_cost_never_blocks_success() {
    let mut openai_server = MockServer::start(vec![MockResponse::json(load_fixture_str(
        "openai/basic_chat_gpt-5.2.json",
    ))]);
    let mut anthropic_server = MockServer::start(vec![MockResponse::json(load_fixture_str(
        "anthropic/basic_chat_claude-opus-4-6.json",
    ))]);
    let mut openrouter_server = MockServer::start(vec![MockResponse::json(load_fixture_str(
        "openrouter/basic_chat_openai.gpt-5.2.json",
    ))]);

    let pricing_table = PricingTable::new(vec![PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5.2*".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
    }]);

    let runtime = runtime_multi_provider(
        vec![
            Arc::new(
                OpenAiAdapter::with_base_url(Some("test-key".to_string()), openai_server.url())
                    .expect("create openai adapter"),
            ),
            Arc::new(
                AnthropicAdapter::with_base_url(
                    Some("test-key".to_string()),
                    anthropic_server.url(),
                )
                .expect("create anthropic adapter"),
            ),
            Arc::new(
                OpenRouterAdapter::with_base_url(
                    Some("test-key".to_string()),
                    openrouter_server.url(),
                )
                .expect("create openrouter adapter"),
            ),
        ],
        Some(pricing_table),
    );

    let openai = runtime
        .run(request_for(Some(ProviderId::Openai), "gpt-5.2", false))
        .await
        .expect("openai run should succeed");
    let anthropic = runtime
        .run(request_for(
            Some(ProviderId::Anthropic),
            "claude-opus-4-6",
            false,
        ))
        .await
        .expect("anthropic run should succeed");
    let openrouter = runtime
        .run(request_for(
            Some(ProviderId::Openrouter),
            "openai/gpt-5.2",
            false,
        ))
        .await
        .expect("openrouter run should succeed");

    assert!(openai.cost.is_some(), "priced provider should get cost");

    assert!(
        anthropic.cost.is_none(),
        "unpriced provider should not get cost"
    );
    assert!(
        anthropic
            .warnings
            .iter()
            .any(|warning| warning.code == "pricing.missing_rule"),
        "anthropic run should emit pricing.missing_rule"
    );

    assert!(
        openrouter.cost.is_none(),
        "unpriced provider should not get cost"
    );
    assert!(
        openrouter
            .warnings
            .iter()
            .any(|warning| warning.code == "pricing.missing_rule"),
        "openrouter run should emit pricing.missing_rule"
    );

    openai_server.shutdown();
    anthropic_server.shutdown();
    openrouter_server.shutdown();
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
