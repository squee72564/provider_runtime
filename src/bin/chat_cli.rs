use std::collections::BTreeMap;
use std::io::{self, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use provider_runtime::ProviderRuntime;
use provider_runtime::core::types::{
    ContentPart, Message, MessageRole, ModelRef, ProviderId, ProviderRequest, ResponseFormat,
    ToolCall, ToolChoice, ToolDefinition, ToolResult, ToolResultContent,
};
use provider_runtime::providers::anthropic::AnthropicAdapter;
use provider_runtime::providers::openai::OpenAiAdapter;
use provider_runtime::providers::openrouter::OpenRouterAdapter;
use serde_json::{Value, json};

const MAX_TOOL_ROUNDS_PER_TURN: usize = 8;

#[derive(Clone, Copy)]
enum CliProvider {
    Openai,
    Anthropic,
    Openrouter,
}

impl CliProvider {
    fn provider_id(self) -> ProviderId {
        match self {
            Self::Openai => ProviderId::Openai,
            Self::Anthropic => ProviderId::Anthropic,
            Self::Openrouter => ProviderId::Openrouter,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Openrouter => "openrouter",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "openrouter" => Some(Self::Openrouter),
            _ => None,
        }
    }
}

struct CliConfig {
    provider: CliProvider,
    model: String,
    max_output_tokens: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    let config = parse_config(std::env::args().skip(1).collect())?;
    let runtime = build_runtime(config.provider)?;

    eprintln!(
        "chat_cli: provider={}, model={}, commands=/exit /quit /clear",
        config.provider.as_str(),
        config.model
    );

    let mut history: Vec<Message> = Vec::new();
    let stdin = io::stdin();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        let bytes = stdin.read_line(&mut input)?;
        if bytes == 0 {
            break;
        }

        let user_text = input.trim();
        if user_text.is_empty() {
            continue;
        }

        if user_text.eq_ignore_ascii_case("/exit") || user_text.eq_ignore_ascii_case("/quit") {
            break;
        }

        if user_text.eq_ignore_ascii_case("/clear") {
            history.clear();
            println!("(history cleared)");
            continue;
        }

        let checkpoint_len = history.len();
        history.push(Message {
            role: MessageRole::User,
            content: vec![ContentPart::Text {
                text: user_text.to_string(),
            }],
        });

        let mut turn_failed = false;
        for _ in 0..MAX_TOOL_ROUNDS_PER_TURN {
            let req = ProviderRequest {
                model: ModelRef {
                    provider_hint: Some(config.provider.provider_id()),
                    model_id: config.model.clone(),
                },
                messages: history.clone(),
                tools: built_in_tools(),
                tool_choice: ToolChoice::Auto,
                response_format: ResponseFormat::Text,
                temperature: None,
                top_p: None,
                max_output_tokens: config.max_output_tokens,
                stop: Vec::new(),
                metadata: BTreeMap::new(),
            };

            let response = match runtime.run(req).await {
                Ok(value) => value,
                Err(err) => {
                    eprintln!("error: {err}");
                    turn_failed = true;
                    break;
                }
            };

            let assistant_content = response.output.content.clone();
            history.push(Message {
                role: MessageRole::Assistant,
                content: assistant_content.clone(),
            });

            let mut printed_any = false;
            let mut tool_calls = Vec::new();
            for part in assistant_content {
                match part {
                    ContentPart::Text { text } => {
                        if !text.trim().is_empty() {
                            println!("{text}");
                            printed_any = true;
                        }
                    }
                    ContentPart::ToolCall { tool_call } => {
                        println!(
                            "[tool_call emitted: id={}, name={}, args={}]",
                            tool_call.id, tool_call.name, tool_call.arguments_json
                        );
                        tool_calls.push(tool_call);
                        printed_any = true;
                    }
                    ContentPart::ToolResult { tool_result } => {
                        println!(
                            "[tool_result echoed: tool_call_id={}]",
                            tool_result.tool_call_id
                        );
                        printed_any = true;
                    }
                }
            }

            if !printed_any {
                println!(
                    "[empty assistant output; finish_reason={:?}]",
                    response.finish_reason
                );
            }

            if tool_calls.is_empty() {
                break;
            }

            for tool_call in tool_calls {
                let tool_result = execute_tool(&tool_call);
                println!(
                    "[tool executed: name={}, tool_call_id={}]",
                    tool_call.name, tool_call.id
                );
                history.push(Message {
                    role: MessageRole::Tool,
                    content: vec![ContentPart::ToolResult { tool_result }],
                });
            }
        }

        if turn_failed {
            history.truncate(checkpoint_len);
        } else if history.len() >= checkpoint_len + MAX_TOOL_ROUNDS_PER_TURN * 2 {
            eprintln!("warning: reached tool loop safety cap ({MAX_TOOL_ROUNDS_PER_TURN} rounds)");
        }
    }

    Ok(())
}

fn built_in_tools() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "time_now".to_string(),
        description: Some("Get the current UNIX timestamp in seconds.".to_string()),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "label": { "type": "string" }
            },
            "required": [],
            "additionalProperties": false
        }),
    }]
}

fn execute_tool(tool_call: &ToolCall) -> ToolResult {
    let output_value = match tool_call.name.as_str() {
        "time_now" => {
            let unix_seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            let label = extract_string_arg(&tool_call.arguments_json, "label");
            json!({
                "unix_seconds": unix_seconds,
                "label": label,
                "source": "chat_cli_builtin_time_now"
            })
        }
        _ => json!({
            "error": format!("unknown tool '{}'", tool_call.name)
        }),
    };

    ToolResult {
        tool_call_id: tool_call.id.clone(),
        content: ToolResultContent::Json {
            value: output_value,
        },
        raw_provider_content: None,
    }
}

fn extract_string_arg(value: &Value, key: &str) -> Option<String> {
    value
        .as_object()
        .and_then(|obj| obj.get(key))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn build_runtime(provider: CliProvider) -> Result<ProviderRuntime, Box<dyn std::error::Error>> {
    let runtime = match provider {
        CliProvider::Openai => ProviderRuntime::builder().with_adapter(Arc::new(
            OpenAiAdapter::new(None).map_err(|e| format!("failed to build OpenAI adapter: {e}"))?,
        )),
        CliProvider::Anthropic => ProviderRuntime::builder().with_adapter(Arc::new(
            AnthropicAdapter::new(None)
                .map_err(|e| format!("failed to build Anthropic adapter: {e}"))?,
        )),
        CliProvider::Openrouter => ProviderRuntime::builder().with_adapter(Arc::new(
            OpenRouterAdapter::new(None)
                .map_err(|e| format!("failed to build OpenRouter adapter: {e}"))?,
        )),
    };

    Ok(runtime.build())
}

fn parse_config(args: Vec<String>) -> Result<CliConfig, Box<dyn std::error::Error>> {
    let mut provider = std::env::var("PROVIDER_RUNTIME_CLI_PROVIDER")
        .ok()
        .and_then(|value| CliProvider::from_str(&value))
        .unwrap_or(CliProvider::Openai);

    let mut model = std::env::var("PROVIDER_RUNTIME_CLI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_model_for(provider).to_string());

    let mut max_output_tokens = std::env::var("PROVIDER_RUNTIME_CLI_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                let value = args
                    .get(i + 1)
                    .ok_or("missing value for --provider (openai|anthropic|openrouter)")?;
                provider = CliProvider::from_str(value)
                    .ok_or("invalid --provider value (expected openai|anthropic|openrouter)")?;
                i += 2;
            }
            "--model" => {
                let value = args
                    .get(i + 1)
                    .ok_or("missing value for --model")?
                    .trim()
                    .to_string();
                if value.is_empty() {
                    return Err("--model must be non-empty".into());
                }
                model = value;
                i += 2;
            }
            "--max-output-tokens" => {
                let value = args
                    .get(i + 1)
                    .ok_or("missing value for --max-output-tokens")?;
                max_output_tokens = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| "--max-output-tokens must be a positive integer")?,
                );
                i += 2;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown argument: {other}").into());
            }
        }
    }

    if model.trim().is_empty() {
        model = default_model_for(provider).to_string();
    }

    Ok(CliConfig {
        provider,
        model,
        max_output_tokens,
    })
}

fn default_model_for(provider: CliProvider) -> &'static str {
    match provider {
        CliProvider::Openai => "gpt-5-mini",
        CliProvider::Anthropic => "claude-sonnet-4-5-20250929",
        CliProvider::Openrouter => "openai/gpt-5-mini",
    }
}

fn print_help() {
    println!(
        "Usage:\n  cargo run --bin chat_cli -- [--provider openai|anthropic|openrouter] [--model MODEL] [--max-output-tokens N]\n\nEnv:\n  OPENAI_API_KEY / ANTHROPIC_API_KEY / OPENROUTER_API_KEY\n  PROVIDER_RUNTIME_CLI_PROVIDER\n  PROVIDER_RUNTIME_CLI_MODEL\n  PROVIDER_RUNTIME_CLI_MAX_OUTPUT_TOKENS\n\nBuilt-in tools:\n  time_now(label?: string)\n\nCommands:\n  /clear   clear conversation history\n  /exit    quit\n  /quit    quit"
    );
}
