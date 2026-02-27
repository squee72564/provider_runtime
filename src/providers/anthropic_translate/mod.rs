use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::core::error::ProviderError;
use crate::core::types::{
    AssistantOutput, ContentPart, FinishReason, MessageRole, ModelInfo, ProviderCapabilities,
    ProviderId, ProviderRequest, ProviderResponse, ResponseFormat, RuntimeWarning, ToolCall,
    ToolChoice, ToolDefinition, ToolResult, ToolResultContent, Usage,
};
use crate::providers::translator_contract::ProviderTranslator;

/*
Anthropic Messages coverage policy (Stage 16/17 strict):
- Mapped fields: model, max_tokens, messages/system, tools, tool_choice, output_config, stop,
  temperature/top_p, metadata.user_id, content blocks, stop_reason, usage.
- Warning-drop fields: unsupported metadata keys, unknown response content block types, parse
  failures for structured output.
- Hard-error fields/states: provider_hint mismatch, empty/invalid model, invalid max_output_tokens,
  invalid sampling/stop/tool schemas and tool ordering, non-object tool_use input, non-prefix
  system messages, malformed payload types.
- Known out-of-scope under frozen canonical model: tool strictness flag, cache-creation token
  breakout, rich server-tool response block typing.
*/

const DEFAULT_MAX_TOKENS: u64 = 1024;

const WARN_BOTH_TEMPERATURE_AND_TOP_P_SET: &str = "both_temperature_and_top_p_set";
const WARN_DROPPED_UNSUPPORTED_METADATA_KEYS: &str = "dropped_unsupported_metadata_keys";
const WARN_DEFAULT_MAX_TOKENS_APPLIED: &str = "default_max_tokens_applied";
const WARN_UNKNOWN_CONTENT_BLOCK_MAPPED: &str = "unknown_content_block_mapped_to_text";
const WARN_UNKNOWN_STOP_REASON: &str = "unknown_stop_reason";
const WARN_USAGE_MISSING: &str = "usage_missing";
const WARN_USAGE_PARTIAL: &str = "usage_partial";
const WARN_STRUCTURED_OUTPUT_PARSE_FAILED: &str = "structured_output_parse_failed";
const WARN_EMPTY_OUTPUT: &str = "empty_output";
const WARN_TOOL_RESULT_COERCED: &str = "tool_result_coerced";
const WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED: &str =
    "tool_result_raw_provider_content_ignored";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnthropicEncodedRequest {
    pub body: Value,
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnthropicDecodeEnvelope {
    pub body: Value,
    pub requested_response_format: ResponseFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnthropicErrorEnvelope {
    pub error_type: Option<String>,
    pub message: String,
    pub request_id: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AnthropicTranslator;

impl ProviderTranslator for AnthropicTranslator {
    type RequestPayload = AnthropicEncodedRequest;
    type ResponsePayload = AnthropicDecodeEnvelope;

    fn encode_request(&self, req: &ProviderRequest) -> Result<Self::RequestPayload, ProviderError> {
        encode_anthropic_request(req)
    }

    fn decode_response(
        &self,
        payload: &Self::ResponsePayload,
    ) -> Result<ProviderResponse, ProviderError> {
        decode_anthropic_response(payload)
    }
}

pub(crate) fn encode_anthropic_request(
    req: &ProviderRequest,
) -> Result<AnthropicEncodedRequest, ProviderError> {
    validate_provider_hint(req)?;
    validate_model_id(req)?;
    validate_max_output_tokens(req)?;
    validate_sampling_controls(req)?;
    validate_stop_sequences(req)?;

    let mut warnings = Vec::new();
    if req.temperature.is_some() && req.top_p.is_some() {
        warnings.push(RuntimeWarning {
            code: WARN_BOTH_TEMPERATURE_AND_TOP_P_SET.to_string(),
            message: "Anthropic recommends setting temperature or top_p, but not both".to_string(),
        });
    }

    let (system, non_system_messages) = map_system_prefix(req)?;
    let mapped_messages = map_non_system_messages(req, &non_system_messages, &mut warnings)?;
    let merged_messages = merge_consecutive_messages(mapped_messages);
    validate_tool_ordering(req, &merged_messages)?;

    if merged_messages.is_empty() {
        return Err(protocol_error(Some(&req.model.model_id), "empty messages"));
    }

    let output_config = map_response_format(req, &merged_messages)?;
    let tools = map_tools(req)?;
    let tool_choice = map_tool_choice(req)?;

    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        Value::String(req.model.model_id.clone()),
    );
    body.insert(
        "max_tokens".to_string(),
        match req.max_output_tokens {
            Some(value) => Value::Number(value.into()),
            None => {
                warnings.push(RuntimeWarning {
                    code: WARN_DEFAULT_MAX_TOKENS_APPLIED.to_string(),
                    message: format!(
                        "max_output_tokens not set; defaulting to {DEFAULT_MAX_TOKENS} for Anthropic"
                    ),
                });
                Value::Number(DEFAULT_MAX_TOKENS.into())
            }
        },
    );

    body.insert(
        "messages".to_string(),
        Value::Array(
            merged_messages
                .into_iter()
                .map(WireMessage::into_json)
                .collect(),
        ),
    );

    if let Some(system_blocks) = system {
        body.insert("system".to_string(), Value::Array(system_blocks));
    }

    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }

    body.insert("tool_choice".to_string(), tool_choice);

    if let Some(output_config) = output_config {
        body.insert("output_config".to_string(), output_config);
    }

    if !req.stop.is_empty() {
        body.insert("stop_sequences".to_string(), json!(req.stop));
    }

    if let Some(temperature) = req.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(top_p) = req.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }

    if let Some(metadata) = map_metadata(req, &mut warnings)? {
        body.insert("metadata".to_string(), metadata);
    }

    Ok(AnthropicEncodedRequest {
        body: Value::Object(body),
        warnings,
    })
}

pub(crate) fn decode_anthropic_response(
    payload: &AnthropicDecodeEnvelope,
) -> Result<ProviderResponse, ProviderError> {
    let root = payload
        .body
        .as_object()
        .ok_or_else(|| protocol_error(None, "anthropic response payload must be a JSON object"))?;

    let model = root
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<unknown-model>")
        .to_string();

    let role = root
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(Some(&model), "anthropic response missing role"))?;
    if role != "assistant" {
        return Err(protocol_error(
            Some(&model),
            format!("anthropic response role must be assistant, got {role}"),
        ));
    }

    let stop_reason = root
        .get("stop_reason")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(Some(&model), "anthropic response missing stop_reason"))?;

    let content_blocks = root
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error(Some(&model), "anthropic response missing content array"))?;

    let mut warnings = Vec::new();
    let mut content = Vec::new();
    let mut text_blocks = Vec::new();

    for block in content_blocks {
        let block_obj = block.as_object().ok_or_else(|| {
            protocol_error(Some(&model), "anthropic content block must be object")
        })?;
        let block_type = block_obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error(Some(&model), "anthropic content block missing type"))?;

        match block_type {
            "text" => {
                let text = block_obj
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        protocol_error(Some(&model), "text content block missing text")
                    })?;
                text_blocks.push(text.to_string());
                content.push(ContentPart::Text {
                    text: text.to_string(),
                });
            }
            "tool_use" => {
                let id = block_obj
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| protocol_error(Some(&model), "tool_use block missing id"))?;
                let name = block_obj
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| protocol_error(Some(&model), "tool_use block missing name"))?;
                let input = block_obj
                    .get("input")
                    .ok_or_else(|| protocol_error(Some(&model), "tool_use block missing input"))?
                    .clone();
                if !input.is_object() {
                    return Err(protocol_error(
                        Some(&model),
                        "tool_use input must be a JSON object",
                    ));
                }
                content.push(ContentPart::ToolCall {
                    tool_call: ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments_json: input,
                    },
                });
            }
            "thinking" | "redacted_thinking" => {}
            _ => {
                warnings.push(RuntimeWarning {
                    code: WARN_UNKNOWN_CONTENT_BLOCK_MAPPED.to_string(),
                    message: format!(
                        "anthropic content block type '{block_type}' mapped to canonical text via JSON"
                    ),
                });
                content.push(ContentPart::Text {
                    text: stable_json_string(block),
                });
            }
        }
    }

    if content.is_empty() {
        warnings.push(RuntimeWarning {
            code: WARN_EMPTY_OUTPUT.to_string(),
            message: "anthropic response contained no content blocks".to_string(),
        });
    }

    let finish_reason = map_finish_reason(stop_reason, &model, &mut warnings)?;
    let usage = decode_usage(root.get("usage"), &model, &mut warnings)?;
    let structured_output = decode_structured_output(
        &payload.requested_response_format,
        &text_blocks,
        &model,
        &mut warnings,
    );

    Ok(ProviderResponse {
        output: AssistantOutput {
            content,
            structured_output,
        },
        usage,
        cost: None,
        provider: ProviderId::Anthropic,
        model,
        raw_provider_response: None,
        finish_reason,
        warnings,
    })
}

pub(crate) fn parse_anthropic_error_envelope(body: &str) -> Option<AnthropicErrorEnvelope> {
    let payload = serde_json::from_str::<Value>(body).ok()?;
    let root = payload.as_object()?;

    let error_obj = root.get("error")?.as_object()?;
    let message = error_obj
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)?;

    let error_type = error_obj
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request_id = root
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    Some(AnthropicErrorEnvelope {
        error_type,
        message,
        request_id,
    })
}

pub(crate) fn format_anthropic_error_message(envelope: &AnthropicErrorEnvelope) -> String {
    match &envelope.error_type {
        Some(error_type) => format!("anthropic error: {} [type={error_type}]", envelope.message),
        None => format!("anthropic error: {}", envelope.message),
    }
}

pub(crate) fn decode_anthropic_models_list(
    payload: &Value,
    capabilities: &ProviderCapabilities,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let root = payload
        .as_object()
        .ok_or_else(|| protocol_error(None, "anthropic models payload must be a JSON object"))?;
    let data = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error(None, "anthropic models payload missing data array"))?;

    let mut discovered = Vec::new();
    let mut seen = BTreeSet::new();

    for (index, item) in data.iter().enumerate() {
        let model_obj = item.as_object().ok_or_else(|| {
            protocol_error(
                None,
                format!("anthropic models payload contains non-object entry at index {index}"),
            )
        })?;

        let model_id = model_obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                protocol_error(
                    None,
                    format!("anthropic models payload entry missing id at index {index}"),
                )
            })?
            .trim()
            .to_string();

        if model_id.is_empty() {
            return Err(protocol_error(
                None,
                format!("anthropic models payload entry has empty id at index {index}"),
            ));
        }

        if !seen.insert(model_id.clone()) {
            continue;
        }

        discovered.push(ModelInfo {
            provider: ProviderId::Anthropic,
            model_id,
            display_name: model_obj
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            context_window: None,
            max_output_tokens: None,
            supports_tools: capabilities.supports_tools,
            supports_structured_output: capabilities.supports_structured_output,
        });
    }

    Ok(discovered)
}

#[derive(Debug, Clone, PartialEq)]
struct WireMessage {
    role: &'static str,
    content: Vec<Value>,
}

impl WireMessage {
    fn into_json(self) -> Value {
        json!({
            "role": self.role,
            "content": self.content,
        })
    }
}

fn validate_provider_hint(req: &ProviderRequest) -> Result<(), ProviderError> {
    if let Some(provider_hint) = &req.model.provider_hint {
        if *provider_hint != ProviderId::Anthropic {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("provider_hint must be Anthropic, got {provider_hint:?}"),
            ));
        }
    }

    Ok(())
}

fn validate_model_id(req: &ProviderRequest) -> Result<(), ProviderError> {
    if req.model.model_id.trim().is_empty() {
        return Err(protocol_error(None, "missing model_id"));
    }

    Ok(())
}

fn validate_max_output_tokens(req: &ProviderRequest) -> Result<(), ProviderError> {
    if req.max_output_tokens == Some(0) {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "max_output_tokens must be at least 1 for Anthropic",
        ));
    }

    Ok(())
}

fn validate_sampling_controls(req: &ProviderRequest) -> Result<(), ProviderError> {
    if let Some(temperature) = req.temperature {
        if !(0.0..=1.0).contains(&temperature) {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("temperature must be in [0.0, 1.0], got {temperature}"),
            ));
        }
    }

    if let Some(top_p) = req.top_p {
        if !(0.0..=1.0).contains(&top_p) {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("top_p must be in [0.0, 1.0], got {top_p}"),
            ));
        }
    }

    Ok(())
}

fn validate_stop_sequences(req: &ProviderRequest) -> Result<(), ProviderError> {
    for stop in &req.stop {
        if stop.is_empty() {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "stop sequences must not contain empty strings",
            ));
        }
    }

    Ok(())
}

fn map_system_prefix(
    req: &ProviderRequest,
) -> Result<(Option<Vec<Value>>, Vec<&crate::core::types::Message>), ProviderError> {
    let mut index = 0;
    while index < req.messages.len() && req.messages[index].role == MessageRole::System {
        index += 1;
    }

    if req.messages[index..]
        .iter()
        .any(|message| message.role == MessageRole::System)
    {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "system messages must form a contiguous prefix for Anthropic",
        ));
    }

    let mut system_blocks = Vec::new();
    for message in &req.messages[..index] {
        for part in &message.content {
            match part {
                ContentPart::Text { text } => system_blocks.push(json!({
                    "type": "text",
                    "text": text,
                })),
                _ => {
                    return Err(protocol_error(
                        Some(&req.model.model_id),
                        "system messages only support text content",
                    ));
                }
            }
        }
    }

    let rest = req.messages[index..].iter().collect::<Vec<_>>();
    let system = if system_blocks.is_empty() {
        None
    } else {
        Some(system_blocks)
    };

    Ok((system, rest))
}

fn map_non_system_messages(
    req: &ProviderRequest,
    messages: &[&crate::core::types::Message],
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Vec<WireMessage>, ProviderError> {
    let mut mapped = Vec::new();
    let mut seen_tool_ids = BTreeSet::new();

    for message in messages {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "user",
            MessageRole::System => unreachable!(),
        };

        let mut blocks = Vec::new();

        for part in &message.content {
            match part {
                ContentPart::Text { text } => match message.role {
                    MessageRole::Tool => {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool messages must contain tool_result content only",
                        ));
                    }
                    _ => blocks.push(json!({ "type": "text", "text": text })),
                },
                ContentPart::ToolCall { tool_call } => {
                    if message.role != MessageRole::Assistant {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool_call content is only valid in assistant messages",
                        ));
                    }
                    if !tool_call.arguments_json.is_object() {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            format!(
                                "tool_call '{}' arguments_json must be a JSON object",
                                tool_call.name
                            ),
                        ));
                    }
                    seen_tool_ids.insert(tool_call.id.clone());
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tool_call.id,
                        "name": tool_call.name,
                        "input": tool_call.arguments_json,
                    }));
                }
                ContentPart::ToolResult { tool_result } => {
                    if message.role != MessageRole::Tool {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool_result content is only valid in tool messages",
                        ));
                    }

                    if !seen_tool_ids.contains(&tool_result.tool_call_id) {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            format!(
                                "tool_result references unknown tool_call_id: {}",
                                tool_result.tool_call_id
                            ),
                        ));
                    }

                    let content = tool_result_content_as_text_blocks(
                        tool_result,
                        &req.model.model_id,
                        warnings,
                    )?;

                    blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_result.tool_call_id,
                        "content": content,
                    }));
                }
            }
        }

        if blocks.is_empty() {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "message content must contain at least one encodable part",
            ));
        }

        mapped.push(WireMessage {
            role,
            content: blocks,
        });
    }

    Ok(mapped)
}

fn tool_result_content_as_text_blocks(
    tool_result: &ToolResult,
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Vec<Value>, ProviderError> {
    if let Some(raw_provider_content) = &tool_result.raw_provider_content {
        if let Some(blocks) = raw_provider_content.as_array() {
            return Ok(blocks.clone());
        }

        warnings.push(RuntimeWarning {
            code: WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED.to_string(),
            message:
                "tool_result raw_provider_content ignored for Anthropic because it is not an array"
                    .to_string(),
        });
    }

    match &tool_result.content {
        ToolResultContent::Text { text } => Ok(vec![json!({
            "type": "text",
            "text": text,
        })]),
        ToolResultContent::Json { value } => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_RESULT_COERCED.to_string(),
                message: "tool_result JSON content coerced to Anthropic text block".to_string(),
            });
            Ok(vec![json!({
                "type": "text",
                "text": stable_json_string(&canonicalize_json(value)),
            })])
        }
        ToolResultContent::Parts { parts } => {
            let mut blocks = Vec::new();

            for part in parts {
                match part {
                    ContentPart::Text { text } => {
                        blocks.push(json!({ "type": "text", "text": text }))
                    }
                    _ => {
                        return Err(protocol_error(
                            Some(model),
                            "tool_result parts content must contain only text parts",
                        ));
                    }
                }
            }

            Ok(blocks)
        }
    }
}

fn merge_consecutive_messages(messages: Vec<WireMessage>) -> Vec<WireMessage> {
    let mut merged: Vec<WireMessage> = Vec::new();

    for message in messages {
        if let Some(last) = merged.last_mut()
            && last.role == message.role
        {
            last.content.extend(message.content);
            if last.role == "user" {
                reorder_user_content_tool_results_first(&mut last.content);
            }
            continue;
        }

        let mut message = message;
        if message.role == "user" {
            reorder_user_content_tool_results_first(&mut message.content);
        }
        merged.push(message);
    }

    merged
}

fn reorder_user_content_tool_results_first(content: &mut Vec<Value>) {
    let mut tool_results = Vec::new();
    let mut others = Vec::new();

    for block in content.drain(..) {
        let is_tool_result = block
            .as_object()
            .and_then(|obj| obj.get("type"))
            .and_then(Value::as_str)
            .map(|value| value == "tool_result")
            .unwrap_or(false);

        if is_tool_result {
            tool_results.push(block);
        } else {
            others.push(block);
        }
    }

    content.extend(tool_results);
    content.extend(others);
}

fn validate_tool_ordering(
    req: &ProviderRequest,
    messages: &[WireMessage],
) -> Result<(), ProviderError> {
    for (index, message) in messages.iter().enumerate() {
        if message.role != "assistant" {
            continue;
        }

        let pending_tool_ids = message
            .content
            .iter()
            .filter_map(|block| {
                let block_obj = block.as_object()?;
                let block_type = block_obj.get("type")?.as_str()?;
                if block_type != "tool_use" {
                    return None;
                }
                block_obj.get("id")?.as_str().map(str::to_string)
            })
            .collect::<Vec<_>>();

        if pending_tool_ids.is_empty() {
            continue;
        }

        let Some(next_message) = messages.get(index + 1) else {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "assistant tool_use requires a following user tool_result message",
            ));
        };

        if next_message.role != "user" {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "assistant tool_use must be followed by a user message containing tool_result blocks",
            ));
        }

        let mut prefix_tool_result_ids = Vec::new();
        for block in &next_message.content {
            let block_obj = block.as_object().ok_or_else(|| {
                protocol_error(
                    Some(&req.model.model_id),
                    "anthropic user content block must be object",
                )
            })?;
            let block_type = block_obj
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    protocol_error(
                        Some(&req.model.model_id),
                        "anthropic user content block missing type",
                    )
                })?;

            if block_type != "tool_result" {
                break;
            }

            let tool_use_id = block_obj
                .get("tool_use_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    protocol_error(
                        Some(&req.model.model_id),
                        "tool_result block missing tool_use_id",
                    )
                })?;
            prefix_tool_result_ids.push(tool_use_id.to_string());
        }

        if prefix_tool_result_ids.is_empty() {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "assistant tool_use requires tool_result blocks at the start of the next user message",
            ));
        }

        for pending_id in pending_tool_ids {
            if !prefix_tool_result_ids.iter().any(|id| id == &pending_id) {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    format!(
                        "missing tool_result for assistant tool_use id '{pending_id}' in following user message"
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn map_tools(req: &ProviderRequest) -> Result<Vec<Value>, ProviderError> {
    let mut tools = Vec::new();

    for tool in &req.tools {
        tools.push(map_tool_definition(tool, &req.model.model_id)?);
    }

    Ok(tools)
}

fn map_tool_definition(tool: &ToolDefinition, model: &str) -> Result<Value, ProviderError> {
    if tool.name.trim().is_empty() {
        return Err(protocol_error(
            Some(model),
            "tool definitions require non-empty names",
        ));
    }

    if tool.name.chars().count() > 128 {
        return Err(protocol_error(
            Some(model),
            format!("tool '{}' name exceeds 128 characters", tool.name),
        ));
    }

    if !tool.parameters_schema.is_object() {
        return Err(protocol_error(
            Some(model),
            format!(
                "tool '{}' parameters_schema must be a JSON object",
                tool.name
            ),
        ));
    }

    let mut mapped = Map::new();
    mapped.insert("name".to_string(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        mapped.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    mapped.insert("input_schema".to_string(), tool.parameters_schema.clone());

    Ok(Value::Object(mapped))
}

fn map_tool_choice(req: &ProviderRequest) -> Result<Value, ProviderError> {
    if req.tools.is_empty() {
        match req.tool_choice {
            ToolChoice::Auto | ToolChoice::None => {}
            ToolChoice::Required | ToolChoice::Specific { .. } => {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "tool_choice requires at least one tool definition",
                ));
            }
        }
    }

    let mapped = match &req.tool_choice {
        ToolChoice::None => json!({ "type": "none" }),
        ToolChoice::Auto => json!({ "type": "auto" }),
        ToolChoice::Required => json!({ "type": "any" }),
        ToolChoice::Specific { name } => {
            if name.trim().is_empty() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "tool_choice specific requires a non-empty tool name",
                ));
            }

            if !req.tools.iter().any(|tool| tool.name == *name) {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    format!("tool_choice specific references unknown tool: {name}"),
                ));
            }

            json!({ "type": "tool", "name": name, "disable_parallel_tool_use": true })
        }
    };

    Ok(mapped)
}

fn map_response_format(
    req: &ProviderRequest,
    messages: &[WireMessage],
) -> Result<Option<Value>, ProviderError> {
    match &req.response_format {
        ResponseFormat::Text => Ok(None),
        ResponseFormat::JsonObject => {
            validate_no_prefill_assistant(req, messages)?;
            Ok(Some(json!({
                "format": {
                    "type": "json_schema",
                    "schema": {
                        "type": "object",
                        "additionalProperties": true
                    }
                }
            })))
        }
        ResponseFormat::JsonSchema { schema, .. } => {
            validate_no_prefill_assistant(req, messages)?;
            Ok(Some(json!({
                "format": {
                    "type": "json_schema",
                    "schema": schema
                }
            })))
        }
    }
}

fn validate_no_prefill_assistant(
    req: &ProviderRequest,
    messages: &[WireMessage],
) -> Result<(), ProviderError> {
    if messages.last().map(|message| message.role) == Some("assistant") {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "json response formats are incompatible with assistant-prefill final messages",
        ));
    }

    Ok(())
}

fn map_metadata(
    req: &ProviderRequest,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Option<Value>, ProviderError> {
    let mut metadata = Map::new();

    if let Some(user_id) = req.metadata.get("user_id") {
        if user_id.chars().count() > 256 {
            return Err(protocol_error(
                Some(&req.model.model_id),
                "metadata.user_id exceeds 256 characters",
            ));
        }
        metadata.insert("user_id".to_string(), Value::String(user_id.clone()));
    }

    if req.metadata.keys().any(|key| key != "user_id") {
        warnings.push(RuntimeWarning {
            code: WARN_DROPPED_UNSUPPORTED_METADATA_KEYS.to_string(),
            message: "anthropic metadata only supports user_id; unsupported keys dropped"
                .to_string(),
        });
    }

    if metadata.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Value::Object(metadata)))
    }
}

fn map_finish_reason(
    stop_reason: &str,
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<FinishReason, ProviderError> {
    let mapped = match stop_reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCalls,
        "refusal" => FinishReason::ContentFilter,
        "pause_turn" => FinishReason::Other,
        other => {
            warnings.push(RuntimeWarning {
                code: WARN_UNKNOWN_STOP_REASON.to_string(),
                message: format!("unknown anthropic stop_reason '{other}' mapped to Other"),
            });
            FinishReason::Other
        }
    };

    if stop_reason.is_empty() {
        return Err(protocol_error(
            Some(model),
            "anthropic stop_reason must not be empty",
        ));
    }

    Ok(mapped)
}

fn decode_usage(
    usage_value: Option<&Value>,
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Usage, ProviderError> {
    let Some(usage_value) = usage_value else {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_MISSING.to_string(),
            message: "anthropic response missing usage object".to_string(),
        });
        return Ok(Usage::default());
    };

    let usage_obj = usage_value
        .as_object()
        .ok_or_else(|| protocol_error(Some(model), "anthropic usage must be a JSON object"))?;

    let input_tokens = parse_usage_u64(usage_obj.get("input_tokens"), model, "input_tokens")?;
    let cache_creation_input_tokens = parse_usage_u64(
        usage_obj.get("cache_creation_input_tokens"),
        model,
        "cache_creation_input_tokens",
    )?;
    let cache_read_input_tokens = parse_usage_u64(
        usage_obj.get("cache_read_input_tokens"),
        model,
        "cache_read_input_tokens",
    )?;
    let output_tokens = parse_usage_u64(usage_obj.get("output_tokens"), model, "output_tokens")?;

    if input_tokens.is_none() || output_tokens.is_none() {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_PARTIAL.to_string(),
            message: "anthropic usage object missing required token fields".to_string(),
        });
    }

    let billed_input = input_tokens.map(|base| {
        base + cache_creation_input_tokens.unwrap_or(0) + cache_read_input_tokens.unwrap_or(0)
    });
    let total_tokens = match (billed_input, output_tokens) {
        (Some(input), Some(output)) => Some(input + output),
        _ => None,
    };

    Ok(Usage {
        input_tokens: billed_input,
        output_tokens,
        cached_input_tokens: cache_read_input_tokens,
        total_tokens,
    })
}

fn parse_usage_u64(
    value: Option<&Value>,
    model: &str,
    field_name: &str,
) -> Result<Option<u64>, ProviderError> {
    match value {
        None => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .ok_or_else(|| {
                protocol_error(
                    Some(model),
                    format!("anthropic usage field '{field_name}' must be an unsigned integer"),
                )
            })
            .map(Some),
        Some(_) => Err(protocol_error(
            Some(model),
            format!("anthropic usage field '{field_name}' must be numeric"),
        )),
    }
}

fn decode_structured_output(
    requested_response_format: &ResponseFormat,
    text_blocks: &[String],
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Option<Value> {
    match requested_response_format {
        ResponseFormat::Text => None,
        ResponseFormat::JsonSchema { .. } => {
            let first_text = text_blocks.first()?;

            parse_json_with_warning(first_text, model, warnings)
        }
        ResponseFormat::JsonObject => {
            if text_blocks.is_empty() {
                warnings.push(RuntimeWarning {
                    code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                    message: "json_object requested but response contained no text blocks"
                        .to_string(),
                });
                return None;
            }

            for text in text_blocks {
                if let Ok(value) = serde_json::from_str::<Value>(text)
                    && value.is_object()
                {
                    return Some(value);
                }
            }

            let combined = text_blocks.join("\n");
            if let Some(object_text) = extract_first_json_object(&combined)
                && let Some(parsed) = parse_json_with_warning(&object_text, model, warnings)
                && parsed.is_object()
            {
                return Some(parsed);
            }

            warnings.push(RuntimeWarning {
                code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                message: "failed to parse json_object structured output from anthropic text blocks"
                    .to_string(),
            });
            None
        }
    }
}

fn parse_json_with_warning(
    text: &str,
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(RuntimeWarning {
                code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                message: format!(
                    "failed to parse structured output JSON for model {model}: {error}"
                ),
            });
            None
        }
    }
}

fn extract_first_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut depth = 0_u64;
    let mut in_string = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().enumerate() {
        let ch = *byte as char;

        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }

        if ch == '{' {
            if start.is_none() {
                start = Some(index);
            }
            depth += 1;
            continue;
        }

        if ch == '}' && depth > 0 {
            depth -= 1;
            if depth == 0 {
                if let Some(start_index) = start {
                    return Some(text[start_index..=index].to_string());
                }
            }
        }
    }

    None
}

fn stable_json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();

            let mut out = Map::new();
            for key in keys {
                let next = map.get(&key).expect("key collected from object must exist");
                out.insert(key, canonicalize_json(next));
            }

            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        _ => value.clone(),
    }
}

fn protocol_error(model: Option<&str>, message: impl Into<String>) -> ProviderError {
    ProviderError::Protocol {
        provider: ProviderId::Anthropic,
        model: model.map(str::to_string),
        request_id: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
