#![cfg(feature = "live-tests")]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Once;

use provider_runtime::ProviderRuntime;
use provider_runtime::core::error::RuntimeError;
use provider_runtime::core::types::{
    ContentPart, DiscoveryOptions, FinishReason, Message, MessageRole, ModelCatalog, ModelInfo,
    ModelRef, ProviderId, ProviderRequest, ProviderResponse, ResponseFormat, ToolCall, ToolChoice,
    ToolDefinition, ToolResult, ToolResultContent,
};
use provider_runtime::handoff::normalize_handoff_messages;
use provider_runtime::pricing::{PriceRule, PricingTable};
use provider_runtime::providers::anthropic::AnthropicAdapter;
use provider_runtime::providers::openai::OpenAiAdapter;
use provider_runtime::providers::openrouter::OpenRouterAdapter;
use serde_json::json;

const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";

const OPENAI_MODEL_ENV: &str = "OPENAI_LIVE_MODEL";
const ANTHROPIC_MODEL_ENV: &str = "ANTHROPIC_LIVE_MODEL";
const OPENROUTER_MODEL_ENV: &str = "OPENROUTER_LIVE_MODEL";

const LIVE_DISCOVERY_ENV: &str = "PROVIDER_RUNTIME_LIVE_DISCOVERY";
const LIVE_FAILURES_ENV: &str = "PROVIDER_RUNTIME_LIVE_FAILURES";

const DEFAULT_OPENAI_MODEL: &str = "gpt-5-mini";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5-20250929";
const DEFAULT_OPENROUTER_MODEL: &str = "openai/gpt-5-mini";

static DOTENV_INIT: Once = Once::new();

#[derive(Debug, Clone, Copy)]
enum LiveProvider {
    Openai,
    Anthropic,
    Openrouter,
}

impl LiveProvider {
    fn id(self) -> ProviderId {
        match self {
            Self::Openai => ProviderId::Openai,
            Self::Anthropic => ProviderId::Anthropic,
            Self::Openrouter => ProviderId::Openrouter,
        }
    }

    fn key_env(self) -> &'static str {
        match self {
            Self::Openai => OPENAI_API_KEY_ENV,
            Self::Anthropic => ANTHROPIC_API_KEY_ENV,
            Self::Openrouter => OPENROUTER_API_KEY_ENV,
        }
    }

    fn model_env(self) -> &'static str {
        match self {
            Self::Openai => OPENAI_MODEL_ENV,
            Self::Anthropic => ANTHROPIC_MODEL_ENV,
            Self::Openrouter => OPENROUTER_MODEL_ENV,
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            Self::Openai => DEFAULT_OPENAI_MODEL,
            Self::Anthropic => DEFAULT_ANTHROPIC_MODEL,
            Self::Openrouter => DEFAULT_OPENROUTER_MODEL,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
        }
    }
}

fn env_non_empty(name: &str) -> Option<String> {
    DOTENV_INIT.call_once(|| {
        let _ = dotenvy::dotenv();
    });

    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn env_flag(name: &str) -> bool {
    matches!(
        env_non_empty(name)
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

fn provider_model(provider: LiveProvider) -> String {
    env_non_empty(provider.model_env()).unwrap_or_else(|| provider.default_model().to_string())
}

fn provider_key(provider: LiveProvider) -> Option<String> {
    env_non_empty(provider.key_env())
}

fn weather_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "weather_lookup".to_string(),
        description: Some("Returns current weather for a city".to_string()),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "city": { "type": "string" }
            },
            "required": ["city"],
            "additionalProperties": false
        }),
    }
}

fn assistant_has_non_empty_text(content: &[ContentPart]) -> bool {
    content
        .iter()
        .any(|part| matches!(part, ContentPart::Text { text } if !text.trim().is_empty()))
}

fn response_has_warning(response: &ProviderResponse, code: &str) -> bool {
    response.warnings.iter().any(|warning| warning.code == code)
}

fn response_debug_summary(response: &ProviderResponse) -> String {
    let mut text_parts = 0usize;
    let mut tool_call_parts = 0usize;
    let mut tool_result_parts = 0usize;
    for part in &response.output.content {
        match part {
            ContentPart::Text { .. } => text_parts += 1,
            ContentPart::ToolCall { .. } => tool_call_parts += 1,
            ContentPart::ToolResult { .. } => tool_result_parts += 1,
        }
    }

    let warning_pairs: Vec<String> = response
        .warnings
        .iter()
        .map(|warning| format!("{}:{}", warning.code, warning.message))
        .collect();

    format!(
        "provider={:?}, model={}, finish_reason={:?}, usage={{input={:?}, output={:?}, total={:?}, cached_input={:?}}}, parts={{text={}, tool_call={}, tool_result={}}}, structured_output_present={}, warnings={:?}",
        response.provider,
        response.model,
        response.finish_reason,
        response.usage.input_tokens,
        response.usage.output_tokens,
        response.usage.total_tokens,
        response.usage.cached_input_tokens,
        text_parts,
        tool_call_parts,
        tool_result_parts,
        response.output.structured_output.is_some(),
        warning_pairs
    )
}

fn first_tool_call(response: &ProviderResponse) -> Option<ToolCall> {
    response.output.content.iter().find_map(|part| {
        if let ContentPart::ToolCall { tool_call } = part {
            Some(tool_call.clone())
        } else {
            None
        }
    })
}

fn assert_usage_parsed(response: &ProviderResponse) {
    assert!(
        response.usage.input_tokens.is_some()
            || response.usage.output_tokens.is_some()
            || response.usage.total_tokens.is_some(),
        "expected parsed usage tokens, got {:?}",
        response.usage
    );
}

fn assert_basic_response_invariants(response: &ProviderResponse, provider: LiveProvider) {
    assert_eq!(response.provider, provider.id());
    assert_ne!(
        response.finish_reason,
        FinishReason::Error,
        "unexpected finish reason: {}",
        response_debug_summary(response)
    );
    assert!(
        assistant_has_non_empty_text(&response.output.content),
        "expected non-empty assistant text content: {}",
        response_debug_summary(response)
    );
    assert_usage_parsed(response);
}

fn assert_tool_call_response_invariants(
    response: &ProviderResponse,
    provider: LiveProvider,
) -> ToolCall {
    assert_eq!(response.provider, provider.id());
    let tool_call = first_tool_call(response).unwrap_or_else(|| {
        panic!(
            "expected at least one tool call: {}",
            response_debug_summary(response)
        )
    });

    assert!(
        !tool_call.id.trim().is_empty(),
        "tool call id must be non-empty"
    );
    assert!(
        !tool_call.name.trim().is_empty(),
        "tool call name must be non-empty"
    );
    assert!(
        tool_call.arguments_json.is_object(),
        "tool call arguments must be a JSON object; got {}",
        tool_call.arguments_json
    );
    assert_usage_parsed(response);

    tool_call
}

fn basic_request(provider: LiveProvider, model_id: &str) -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(provider.id()),
            model_id: model_id.to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Reply with one short sentence confirming live smoke test success."
                    .to_string(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: Some(max_tokens_for(provider, LiveScenario::Basic)),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn tool_call_request(provider: LiveProvider, model_id: &str) -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(provider.id()),
            model_id: model_id.to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Call exactly one tool named weather_lookup with city set to \"Boston\" before any text response. Do not answer with plain text before the tool call.".to_string(),
            }],
        }],
        tools: vec![weather_tool_definition()],
        tool_choice: ToolChoice::Required,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: Some(max_tokens_for(provider, LiveScenario::ToolCall)),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn tool_result_roundtrip_request(
    provider: LiveProvider,
    model_id: &str,
    tool_call: ToolCall,
) -> ProviderRequest {
    let tool_call_id = tool_call.id.clone();

    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(provider.id()),
            model_id: model_id.to_string(),
        },
        messages: vec![
            Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text {
                    text: "Use the provided tool result and return exactly one short weather summary sentence."
                        .to_string(),
                }],
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall { tool_call }],
            },
            Message {
                role: MessageRole::Tool,
                content: vec![ContentPart::ToolResult {
                    tool_result: ToolResult {
                        tool_call_id,
                        content: ToolResultContent::Json {
                            value: json!({
                                "city": "Boston",
                                "temperature_f": 68,
                                "conditions": "sunny"
                            }),
                        },
                        raw_provider_content: None,
                    },
                }],
            },
        ],
        tools: vec![weather_tool_definition()],
        tool_choice: ToolChoice::Required,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: Some(max_tokens_for(provider, LiveScenario::ToolRoundtrip)),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn structured_output_request(provider: LiveProvider, model_id: &str) -> ProviderRequest {
    ProviderRequest {
        model: ModelRef {
            provider_hint: Some(provider.id()),
            model_id: model_id.to_string(),
        },
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Return JSON containing city and forecast fields for Boston.".to_string(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::JsonSchema {
            name: "weather_schema".to_string(),
            schema: json!({
                "type": "object",
                "properties": {
                    "city": { "type": "string" },
                    "forecast": { "type": "string" }
                },
                "required": ["city", "forecast"],
                "additionalProperties": false
            }),
        },
        temperature: None,
        top_p: None,
        max_output_tokens: Some(max_tokens_for(provider, LiveScenario::Structured)),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

#[derive(Debug, Clone, Copy)]
enum LiveScenario {
    Basic,
    ToolCall,
    ToolRoundtrip,
    Structured,
    Handoff,
}

fn max_tokens_for(provider: LiveProvider, scenario: LiveScenario) -> u32 {
    match provider {
        LiveProvider::Openai => match scenario {
            LiveScenario::Basic => 512,
            LiveScenario::ToolCall => 1024,
            LiveScenario::ToolRoundtrip => 1024,
            LiveScenario::Structured => 1024,
            LiveScenario::Handoff => 1024,
        },
        LiveProvider::Anthropic | LiveProvider::Openrouter => match scenario {
            LiveScenario::Basic => 256,
            LiveScenario::ToolCall => 512,
            LiveScenario::ToolRoundtrip => 512,
            LiveScenario::Structured => 512,
            LiveScenario::Handoff => 512,
        },
    }
}

fn provider_catalog_entry(provider: LiveProvider, model_id: &str) -> ModelInfo {
    ModelInfo {
        provider: provider.id(),
        model_id: model_id.to_string(),
        display_name: Some(format!("{} live model", provider.as_str())),
        context_window: None,
        max_output_tokens: None,
        supports_tools: true,
        supports_structured_output: true,
    }
}

fn adapter_for_provider(
    provider: LiveProvider,
    api_key: String,
) -> Arc<dyn provider_runtime::core::traits::ProviderAdapter> {
    match provider {
        LiveProvider::Openai => {
            Arc::new(OpenAiAdapter::new(Some(api_key)).expect("create openai adapter"))
        }
        LiveProvider::Anthropic => {
            Arc::new(AnthropicAdapter::new(Some(api_key)).expect("create anthropic adapter"))
        }
        LiveProvider::Openrouter => {
            Arc::new(OpenRouterAdapter::new(Some(api_key)).expect("create openrouter adapter"))
        }
    }
}

fn runtime_for_providers(
    providers: &[LiveProvider],
    pricing_table: Option<PricingTable>,
) -> Option<(ProviderRuntime, BTreeMap<String, String>)> {
    let mut catalog_models = Vec::new();
    let mut adapters = Vec::new();
    let mut model_map = BTreeMap::new();

    for provider in providers {
        let Some(api_key) = provider_key(*provider) else {
            eprintln!(
                "skipping test: missing {} for provider {}",
                provider.key_env(),
                provider.as_str()
            );
            return None;
        };

        let model = provider_model(*provider);
        model_map.insert(provider.as_str().to_string(), model.clone());
        catalog_models.push(provider_catalog_entry(*provider, &model));
        adapters.push(adapter_for_provider(*provider, api_key));
    }

    let mut builder = ProviderRuntime::builder().with_model_catalog(ModelCatalog {
        models: catalog_models,
    });

    for adapter in adapters {
        builder = builder.with_adapter(adapter);
    }

    if let Some(table) = pricing_table {
        builder = builder.with_pricing_table(table);
    }

    Some((builder.build(), model_map))
}

async fn run_basic_smoke(provider: LiveProvider) {
    let Some((runtime, models)) = runtime_for_providers(&[provider], None) else {
        return;
    };

    let model = models
        .get(provider.as_str())
        .expect("model mapping should contain provider");
    let response = runtime
        .run(basic_request(provider, model))
        .await
        .expect("basic live request should succeed");

    assert_basic_response_invariants(&response, provider);
}

async fn run_tool_call_smoke(provider: LiveProvider) {
    let Some((runtime, models)) = runtime_for_providers(&[provider], None) else {
        return;
    };

    let model = models
        .get(provider.as_str())
        .expect("model mapping should contain provider");
    let response = runtime
        .run(tool_call_request(provider, model))
        .await
        .expect("tool-call live request should succeed");

    let _tool_call = assert_tool_call_response_invariants(&response, provider);
}

async fn run_tool_result_roundtrip(provider: LiveProvider) {
    let Some((runtime, models)) = runtime_for_providers(&[provider], None) else {
        return;
    };

    let model = models
        .get(provider.as_str())
        .expect("model mapping should contain provider");

    let initial = runtime
        .run(tool_call_request(provider, model))
        .await
        .expect("roundtrip first call should succeed");
    let tool_call = assert_tool_call_response_invariants(&initial, provider);

    let followup = runtime
        .run(tool_result_roundtrip_request(provider, model, tool_call))
        .await
        .expect("roundtrip follow-up should succeed");

    assert_eq!(followup.provider, provider.id());
    assert_usage_parsed(&followup);
    assert!(
        assistant_has_non_empty_text(&followup.output.content)
            || first_tool_call(&followup).is_some(),
        "expected textual answer or another tool call after tool result"
    );
}

async fn run_structured_output_smoke(provider: LiveProvider) {
    let Some((runtime, models)) = runtime_for_providers(&[provider], None) else {
        return;
    };

    let model = models
        .get(provider.as_str())
        .expect("model mapping should contain provider");

    let response = runtime
        .run(structured_output_request(provider, model))
        .await
        .expect("structured live request should succeed");

    assert_eq!(response.provider, provider.id());
    assert_usage_parsed(&response);

    if let Some(structured) = &response.output.structured_output {
        assert!(
            structured.is_object(),
            "structured_output should be an object when present; got {structured}"
        );
    } else {
        let warned = response_has_warning(&response, "structured_output_parse_failed");
        let text_fallback = assistant_has_non_empty_text(&response.output.content);
        assert!(
            warned || text_fallback,
            "expected structured_output, parse warning, or non-empty text fallback; finish_reason={:?}, warnings={:?}",
            response.finish_reason,
            response
                .warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>()
        );
    }
}

async fn run_cross_provider_handoff(source: LiveProvider, target: LiveProvider) {
    let Some((runtime, models)) = runtime_for_providers(&[source, target], None) else {
        return;
    };

    let source_model = models
        .get(source.as_str())
        .expect("source model mapping should exist");
    let target_model = models
        .get(target.as_str())
        .expect("target model mapping should exist");

    let source_response = runtime
        .run(tool_call_request(source, source_model))
        .await
        .expect("source tool-call request should succeed");
    let source_tool_call = assert_tool_call_response_invariants(&source_response, source);

    let handoff_messages = vec![
        Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: "Use the tool output and return exactly one short weather summary sentence in plain text."
                    .to_string(),
            }],
        },
        Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::ToolCall {
                tool_call: source_tool_call.clone(),
            }],
        },
        Message {
            role: MessageRole::Tool,
            content: vec![ContentPart::ToolResult {
                tool_result: ToolResult {
                    tool_call_id: source_tool_call.id,
                    content: ToolResultContent::Json {
                        value: json!({
                            "city": "Boston",
                            "temperature_f": 68,
                            "conditions": "sunny"
                        }),
                    },
                    raw_provider_content: None,
                },
            }],
        },
    ];

    let normalized = normalize_handoff_messages(&handoff_messages, &target.id());

    let request = ProviderRequest {
        model: ModelRef {
            provider_hint: Some(target.id()),
            model_id: target_model.to_string(),
        },
        messages: normalized,
        tools: vec![weather_tool_definition()],
        tool_choice: ToolChoice::Auto,
        response_format: ResponseFormat::Text,
        temperature: None,
        top_p: None,
        max_output_tokens: Some(max_tokens_for(target, LiveScenario::Handoff)),
        stop: Vec::new(),
        metadata: BTreeMap::new(),
    };

    let target_response = runtime
        .run(request)
        .await
        .expect("target handoff request should succeed");

    assert_eq!(target_response.provider, target.id());
    assert_usage_parsed(&target_response);
    assert!(
        assistant_has_non_empty_text(&target_response.output.content)
            || first_tool_call(&target_response).is_some(),
        "expected text or follow-up tool call from target provider: {}",
        response_debug_summary(&target_response)
    );
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openai_basic_text_smoke() {
    run_basic_smoke(LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_anthropic_basic_text_smoke() {
    run_basic_smoke(LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openrouter_basic_text_smoke() {
    run_basic_smoke(LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openai_tool_call_smoke() {
    run_tool_call_smoke(LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_anthropic_tool_call_smoke() {
    run_tool_call_smoke(LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openrouter_tool_call_smoke() {
    run_tool_call_smoke(LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openai_tool_result_roundtrip() {
    run_tool_result_roundtrip(LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_anthropic_tool_result_roundtrip() {
    run_tool_result_roundtrip(LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openrouter_tool_result_roundtrip() {
    run_tool_result_roundtrip(LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openai_structured_output_smoke() {
    run_structured_output_smoke(LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_anthropic_structured_output_smoke() {
    run_structured_output_smoke(LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openrouter_structured_output_smoke() {
    run_structured_output_smoke(LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_openai_to_anthropic() {
    run_cross_provider_handoff(LiveProvider::Openai, LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_openai_to_openrouter() {
    run_cross_provider_handoff(LiveProvider::Openai, LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_anthropic_to_openai() {
    run_cross_provider_handoff(LiveProvider::Anthropic, LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_anthropic_to_openrouter() {
    run_cross_provider_handoff(LiveProvider::Anthropic, LiveProvider::Openrouter).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_openrouter_to_openai() {
    run_cross_provider_handoff(LiveProvider::Openrouter, LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_handoff_openrouter_to_anthropic() {
    run_cross_provider_handoff(LiveProvider::Openrouter, LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_partial_pricing_non_blocking() {
    let openai_model = provider_model(LiveProvider::Openai);
    let pricing_table = PricingTable::new(vec![PriceRule {
        provider: ProviderId::Openai,
        model_pattern: format!("{openai_model}*"),
        input_cost_per_token: 0.00001,
        output_cost_per_token: 0.00002,
    }]);

    let Some((runtime, models)) = runtime_for_providers(
        &[
            LiveProvider::Openai,
            LiveProvider::Anthropic,
            LiveProvider::Openrouter,
        ],
        Some(pricing_table),
    ) else {
        return;
    };

    let openai_response = runtime
        .run(basic_request(
            LiveProvider::Openai,
            models
                .get(LiveProvider::Openai.as_str())
                .expect("openai model should exist"),
        ))
        .await
        .expect("openai priced request should succeed");
    let anthropic_response = runtime
        .run(basic_request(
            LiveProvider::Anthropic,
            models
                .get(LiveProvider::Anthropic.as_str())
                .expect("anthropic model should exist"),
        ))
        .await
        .expect("anthropic unpriced request should succeed");
    let openrouter_response = runtime
        .run(basic_request(
            LiveProvider::Openrouter,
            models
                .get(LiveProvider::Openrouter.as_str())
                .expect("openrouter model should exist"),
        ))
        .await
        .expect("openrouter unpriced request should succeed");

    assert!(
        openai_response.cost.is_some(),
        "openai should be priced: {}",
        response_debug_summary(&openai_response)
    );

    assert!(
        anthropic_response.cost.is_none(),
        "anthropic should be unpriced"
    );
    assert!(
        response_has_warning(&anthropic_response, "pricing.missing_rule"),
        "anthropic should emit pricing.missing_rule"
    );

    assert!(
        openrouter_response.cost.is_none(),
        "openrouter should be unpriced"
    );
    assert!(
        response_has_warning(&openrouter_response, "pricing.missing_rule"),
        "openrouter should emit pricing.missing_rule"
    );
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_discover_models_smoke() {
    if !env_flag(LIVE_DISCOVERY_ENV) {
        eprintln!("skipping discovery smoke: set {LIVE_DISCOVERY_ENV}=1 to enable");
        return;
    }

    let providers = [
        LiveProvider::Openai,
        LiveProvider::Anthropic,
        LiveProvider::Openrouter,
    ];

    let enabled: Vec<LiveProvider> = providers
        .into_iter()
        .filter(|provider| provider_key(*provider).is_some())
        .collect();

    if enabled.is_empty() {
        eprintln!("skipping discovery smoke: no provider API keys were set");
        return;
    }

    let Some((runtime, _)) = runtime_for_providers(&enabled, None) else {
        return;
    };

    for provider in enabled {
        let catalog = runtime
            .discover_models(DiscoveryOptions {
                remote: true,
                include_provider: vec![provider.id()],
                refresh_cache: true,
            })
            .await
            .expect("discover_models should succeed");

        let discovered_for_provider: Vec<&ModelInfo> = catalog
            .models
            .iter()
            .filter(|model| model.provider == provider.id())
            .collect();

        assert!(
            !discovered_for_provider.is_empty(),
            "expected discovered models for {}",
            provider.as_str()
        );
        assert!(
            discovered_for_provider
                .iter()
                .all(|model| !model.model_id.trim().is_empty()),
            "expected discovered models to have non-empty model ids"
        );
    }
}

fn assert_invalid_key_error(error: RuntimeError, provider: ProviderId) {
    match error {
        RuntimeError::ProviderProtocolError {
            provider: actual_provider,
            message,
            ..
        } => {
            assert_eq!(actual_provider, Some(provider));
            let lower = message.to_ascii_lowercase();
            assert!(
                lower.contains("auth")
                    || lower.contains("invalid")
                    || lower.contains("unauthorized")
                    || lower.contains("forbidden")
                    || lower.contains("api key"),
                "expected auth-like error message; got: {message}"
            );
        }
        other => panic!("expected RuntimeError::ProviderProtocolError, got {other:?}"),
    }
}

async fn run_invalid_key_failure_check(provider: LiveProvider) {
    if !env_flag(LIVE_FAILURES_ENV) {
        eprintln!("skipping failure smoke: set {LIVE_FAILURES_ENV}=1 to enable");
        return;
    }

    let bogus_key = "provider-runtime-live-smoke-invalid-key".to_string();
    let model = provider_model(provider);
    let adapter = adapter_for_provider(provider, bogus_key);

    let runtime = ProviderRuntime::builder()
        .with_model_catalog(ModelCatalog {
            models: vec![provider_catalog_entry(provider, &model)],
        })
        .with_adapter(adapter)
        .build();

    let error = runtime
        .run(basic_request(provider, &model))
        .await
        .expect_err("invalid key live request should fail");

    assert_invalid_key_error(error, provider.id());
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openai_invalid_key_maps_error() {
    run_invalid_key_failure_check(LiveProvider::Openai).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_anthropic_invalid_key_maps_error() {
    run_invalid_key_failure_check(LiveProvider::Anthropic).await;
}

#[tokio::test]
#[ignore = "live network + cost"]
async fn live_openrouter_invalid_key_maps_error() {
    run_invalid_key_failure_check(LiveProvider::Openrouter).await;
}
