use serde_json::{Map, Value, json};

use crate::core::error::ProviderError;
use crate::core::types::{
    AssistantOutput, ContentPart, FinishReason, Message, MessageRole, ModelInfo,
    ProviderCapabilities, ProviderId, ProviderRequest, ProviderResponse, ResponseFormat,
    RuntimeWarning, ToolCall, ToolChoice, ToolDefinition, ToolResult, ToolResultContent, Usage,
};
use crate::providers::translator_contract::ProviderTranslator;

const WARN_BOTH_TEMPERATURE_AND_TOP_P_SET: &str = "both_temperature_and_top_p_set";
const WARN_TOOL_SCHEMA_NOT_STRICT_COMPATIBLE: &str =
    "tool_schema_not_strict_compatible_strict_disabled";
const WARN_TOOL_ARGUMENTS_INVALID_JSON: &str = "tool_arguments_invalid_json";
const WARN_USAGE_MISSING: &str = "usage_missing";
const WARN_MODEL_REFUSAL: &str = "model_refusal";
const WARN_STRUCTURED_OUTPUT_PARSE_FAILED: &str = "structured_output_parse_failed";
const WARN_OPENAI_INCOMPLETE_MAX_OUTPUT_TOKENS: &str = "openai_incomplete_max_output_tokens";
const WARN_OPENAI_INCOMPLETE_CONTENT_FILTER: &str = "openai_incomplete_content_filter";
const WARN_OPENAI_INCOMPLETE_UNKNOWN_REASON: &str = "openai_incomplete_unknown_reason";
const WARN_OPENAI_INCOMPLETE_MISSING_REASON: &str = "openai_incomplete_missing_reason";
const WARN_EMPTY_OUTPUT: &str = "empty_output";
const WARN_TOOL_RESULT_COERCED: &str = "tool_result_coerced";
const WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED: &str =
    "tool_result_raw_provider_content_ignored";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenAiEncodedRequest {
    pub body: Value,
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenAiDecodeEnvelope {
    pub body: Value,
    pub requested_response_format: ResponseFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiErrorEnvelope {
    pub message: String,
    pub code: Option<String>,
    pub error_type: Option<String>,
    pub param: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct OpenAiTranslator;

impl ProviderTranslator for OpenAiTranslator {
    type RequestPayload = OpenAiEncodedRequest;
    type ResponsePayload = OpenAiDecodeEnvelope;

    fn encode_request(&self, req: &ProviderRequest) -> Result<Self::RequestPayload, ProviderError> {
        encode_openai_request(req)
    }

    fn decode_response(
        &self,
        payload: &Self::ResponsePayload,
    ) -> Result<ProviderResponse, ProviderError> {
        decode_openai_response(payload)
    }
}

pub(crate) fn encode_openai_request(
    req: &ProviderRequest,
) -> Result<OpenAiEncodedRequest, ProviderError> {
    validate_provider_hint(req)?;
    validate_model_id(req)?;
    validate_stop(req)?;
    validate_metadata(req)?;
    validate_sampling_controls(req)?;

    let mut warnings = Vec::new();
    if req.temperature.is_some() && req.top_p.is_some() {
        warnings.push(RuntimeWarning {
            code: WARN_BOTH_TEMPERATURE_AND_TOP_P_SET.to_string(),
            message: "OpenAI recommends setting temperature or top_p, but not both".to_string(),
        });
    }

    let text_format = map_response_format(req)?;
    let tool_choice = map_tool_choice(req)?;
    let tools = map_tools(req, &mut warnings)?;
    let input = map_messages(req, &mut warnings)?;

    if input.is_empty() {
        return Err(protocol_error(Some(&req.model.model_id), "empty input"));
    }

    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        Value::String(req.model.model_id.clone()),
    );
    body.insert("store".to_string(), Value::Bool(false));
    body.insert("input".to_string(), Value::Array(input));
    body.insert("text".to_string(), json!({ "format": text_format }));
    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    body.insert("tool_choice".to_string(), tool_choice);

    if let Some(temperature) = req.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = req.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }
    if let Some(max_output_tokens) = req.max_output_tokens {
        body.insert("max_output_tokens".to_string(), json!(max_output_tokens));
    }
    if !req.metadata.is_empty() {
        body.insert("metadata".to_string(), json!(req.metadata));
    }

    Ok(OpenAiEncodedRequest {
        body: Value::Object(body),
        warnings,
    })
}

pub(crate) fn decode_openai_response(
    payload: &OpenAiDecodeEnvelope,
) -> Result<ProviderResponse, ProviderError> {
    let root = payload
        .body
        .as_object()
        .ok_or_else(|| protocol_error(None, "openai response payload must be a JSON object"))?;

    if let Some(error) = parse_openai_error_value(root) {
        return Err(protocol_error(None, format_openai_error_message(&error)));
    }

    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(None, "openai response missing status"))?;

    if status == "failed" {
        return Err(protocol_error(None, "openai response status is failed"));
    }

    if status == "queued" || status == "in_progress" {
        return Err(protocol_error(
            None,
            format!("openai response status is non-terminal: {status}"),
        ));
    }

    let model = root
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<unknown-model>")
        .to_string();

    let mut warnings = Vec::new();
    let mut content = Vec::new();

    let output_items = root
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for item in output_items {
        decode_output_item(&item, &mut content, &mut warnings)?;
    }

    if content.is_empty() {
        warnings.push(RuntimeWarning {
            code: WARN_EMPTY_OUTPUT.to_string(),
            message: "openai response contained no decodable output content".to_string(),
        });
    }

    let structured_output = decode_structured_output(
        &payload.requested_response_format,
        &content,
        &mut warnings,
        Some(&model),
    );
    let usage = decode_usage(root.get("usage"), &mut warnings);

    let incomplete_reason = root
        .get("incomplete_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("reason"))
        .and_then(Value::as_str);

    let finish_reason = map_finish_reason(status, incomplete_reason, &content, &mut warnings)?;

    Ok(ProviderResponse {
        output: AssistantOutput {
            content,
            structured_output,
        },
        usage,
        cost: None,
        provider: ProviderId::Openai,
        model,
        raw_provider_response: None,
        finish_reason,
        warnings,
    })
}

pub(crate) fn parse_openai_error_envelope(body: &str) -> Option<OpenAiErrorEnvelope> {
    let payload = serde_json::from_str::<Value>(body).ok()?;
    let root = payload.as_object()?;
    parse_openai_error_value(root)
}

pub(crate) fn format_openai_error_message(envelope: &OpenAiErrorEnvelope) -> String {
    let mut context = Vec::new();

    if let Some(code) = &envelope.code {
        context.push(format!("code={code}"));
    }
    if let Some(error_type) = &envelope.error_type {
        context.push(format!("type={error_type}"));
    }
    if let Some(param) = &envelope.param {
        context.push(format!("param={param}"));
    }

    if context.is_empty() {
        format!("openai error: {}", envelope.message)
    } else {
        format!(
            "openai error: {} [{}]",
            envelope.message,
            context.join(", ")
        )
    }
}

pub(crate) fn decode_openai_models_list(
    payload: &Value,
    capabilities: &ProviderCapabilities,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let root = payload
        .as_object()
        .ok_or_else(|| protocol_error(None, "openai models payload must be a JSON object"))?;
    let models = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error(None, "openai models payload missing data array"))?;

    let mut discovered = Vec::new();
    for (index, entry) in models.iter().enumerate() {
        let model = entry.as_object().ok_or_else(|| {
            protocol_error(
                None,
                format!("openai models payload contains non-object entry at index {index}"),
            )
        })?;

        let model_id = model.get("id").and_then(Value::as_str).ok_or_else(|| {
            protocol_error(
                None,
                format!("openai models payload entry missing id at index {index}"),
            )
        })?;
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(protocol_error(
                None,
                format!("openai models payload entry has empty id at index {index}"),
            ));
        }

        if discovered
            .iter()
            .any(|candidate: &ModelInfo| candidate.model_id == model_id)
        {
            continue;
        }

        discovered.push(ModelInfo {
            provider: ProviderId::Openai,
            model_id: model_id.to_string(),
            display_name: None,
            context_window: None,
            max_output_tokens: None,
            supports_tools: capabilities.supports_tools,
            supports_structured_output: capabilities.supports_structured_output,
        });
    }

    Ok(discovered)
}

fn validate_provider_hint(req: &ProviderRequest) -> Result<(), ProviderError> {
    if let Some(provider_hint) = &req.model.provider_hint {
        if *provider_hint != ProviderId::Openai {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("provider_hint must be Openai, got {provider_hint:?}"),
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

fn validate_stop(req: &ProviderRequest) -> Result<(), ProviderError> {
    if req.stop.is_empty() {
        return Ok(());
    }

    Err(protocol_error(
        Some(&req.model.model_id),
        "stop sequences are unsupported by OpenAI Responses API",
    ))
}

fn validate_metadata(req: &ProviderRequest) -> Result<(), ProviderError> {
    if req.metadata.len() > 16 {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "metadata supports at most 16 entries",
        ));
    }

    for (key, value) in &req.metadata {
        if key.chars().count() > 64 {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("metadata key exceeds 64 characters: {key}"),
            ));
        }
        if value.chars().count() > 512 {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("metadata value exceeds 512 characters for key: {key}"),
            ));
        }
    }

    Ok(())
}

fn validate_sampling_controls(req: &ProviderRequest) -> Result<(), ProviderError> {
    if let Some(temperature) = req.temperature {
        if !(0.0..=2.0).contains(&temperature) {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("temperature must be in [0.0, 2.0], got {temperature}"),
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

fn map_response_format(req: &ProviderRequest) -> Result<Value, ProviderError> {
    match &req.response_format {
        ResponseFormat::Text => Ok(json!({ "type": "text" })),
        ResponseFormat::JsonObject => {
            if !contains_json_keyword(&req.messages) {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "json_object response format requires the string 'JSON' in message text",
                ));
            }
            Ok(json!({ "type": "json_object" }))
        }
        ResponseFormat::JsonSchema { name, schema } => {
            if name.trim().is_empty() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "json_schema response format requires a non-empty name",
                ));
            }

            Ok(json!({
                "type": "json_schema",
                "name": name,
                "schema": schema,
                "strict": true
            }))
        }
    }
}

fn contains_json_keyword(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        message.content.iter().any(|part| match part {
            ContentPart::Text { text } => text.contains("JSON"),
            _ => false,
        })
    })
}

fn map_tool_choice(req: &ProviderRequest) -> Result<Value, ProviderError> {
    match &req.tool_choice {
        ToolChoice::None => Ok(Value::String("none".to_string())),
        ToolChoice::Auto => Ok(Value::String("auto".to_string())),
        ToolChoice::Required => Ok(Value::String("required".to_string())),
        ToolChoice::Specific { name } => {
            if name.trim().is_empty() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "tool_choice specific requires a non-empty tool name",
                ));
            }

            let found = req.tools.iter().any(|tool| tool.name == *name);
            if !found {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    format!("tool_choice specific references unknown tool: {name}"),
                ));
            }

            Ok(json!({ "type": "function", "name": name }))
        }
    }
}

fn map_tools(
    req: &ProviderRequest,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Vec<Value>, ProviderError> {
    let mut tools = Vec::new();

    for tool in &req.tools {
        tools.push(map_tool_definition(tool, &req.model.model_id, warnings)?);
    }

    Ok(tools)
}

fn map_tool_definition(
    tool: &ToolDefinition,
    model_id: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Value, ProviderError> {
    if tool.name.trim().is_empty() {
        return Err(protocol_error(
            Some(model_id),
            "tool definitions require non-empty names",
        ));
    }

    if !tool.parameters_schema.is_object() {
        return Err(protocol_error(
            Some(model_id),
            format!(
                "tool '{}' parameters_schema must be a JSON object",
                tool.name
            ),
        ));
    }

    let strict = is_strict_compatible_schema(&tool.parameters_schema);
    if !strict {
        warnings.push(RuntimeWarning {
            code: WARN_TOOL_SCHEMA_NOT_STRICT_COMPATIBLE.to_string(),
            message: format!(
                "tool '{}' schema is not strict-compatible; strict disabled",
                tool.name
            ),
        });
    }

    let mut payload = Map::new();
    payload.insert("type".to_string(), Value::String("function".to_string()));
    payload.insert("name".to_string(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        payload.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    payload.insert("parameters".to_string(), tool.parameters_schema.clone());
    payload.insert("strict".to_string(), Value::Bool(strict));

    Ok(Value::Object(payload))
}

fn map_messages(
    req: &ProviderRequest,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Vec<Value>, ProviderError> {
    let mut input_items = Vec::new();
    let mut seen_tool_call_ids: Vec<String> = Vec::new();

    for message in &req.messages {
        let mut message_parts = Vec::new();

        for part in &message.content {
            match part {
                ContentPart::Text { text } => {
                    if message.role == MessageRole::Tool {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool role messages cannot contain plain text content",
                        ));
                    }

                    let part_type = if message.role == MessageRole::Assistant {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    message_parts.push(json!({ "type": part_type, "text": text }));
                }
                ContentPart::ToolCall { tool_call } => {
                    if message.role != MessageRole::Assistant {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool_call content is only valid for assistant role messages",
                        ));
                    }

                    flush_message_item(&mut input_items, &message.role, &mut message_parts);

                    let arguments =
                        serde_json::to_string(&tool_call.arguments_json).map_err(|e| {
                            ProviderError::Serialization {
                                provider: ProviderId::Openai,
                                model: Some(req.model.model_id.clone()),
                                request_id: None,
                                message: format!(
                                    "failed to serialize tool_call arguments for '{}': {e}",
                                    tool_call.name
                                ),
                            }
                        })?;

                    seen_tool_call_ids.push(tool_call.id.clone());
                    input_items.push(json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": arguments
                    }));
                }
                ContentPart::ToolResult { tool_result } => {
                    if message.role != MessageRole::Tool {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool_result content is only valid for tool role messages",
                        ));
                    }

                    flush_message_item(&mut input_items, &message.role, &mut message_parts);

                    if !seen_tool_call_ids.contains(&tool_result.tool_call_id) {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            format!(
                                "tool_result_without_matching_tool_call: {}",
                                tool_result.tool_call_id
                            ),
                        ));
                    }

                    let output = serialize_tool_result_output(tool_result, req, warnings)?;
                    input_items.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_result.tool_call_id,
                        "output": output
                    }));
                }
            }
        }

        flush_message_item(&mut input_items, &message.role, &mut message_parts);
    }

    Ok(input_items)
}

fn flush_message_item(
    input_items: &mut Vec<Value>,
    role: &MessageRole,
    message_parts: &mut Vec<Value>,
) {
    if message_parts.is_empty() {
        return;
    }

    let role_value = match role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => return,
    };

    let content = std::mem::take(message_parts);
    input_items.push(json!({
        "type": "message",
        "role": role_value,
        "content": content
    }));
}

fn serialize_tool_result_output(
    tool_result: &ToolResult,
    req: &ProviderRequest,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<String, ProviderError> {
    if let Some(raw_provider_content) = &tool_result.raw_provider_content {
        if let Some(raw_text) = raw_provider_content.as_str() {
            return Ok(raw_text.to_string());
        }

        warnings.push(RuntimeWarning {
            code: WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED.to_string(),
            message:
                "tool_result raw_provider_content ignored for OpenAI because it is not a string"
                    .to_string(),
        });
    }

    match &tool_result.content {
        ToolResultContent::Text { text } => Ok(text.clone()),
        ToolResultContent::Json { value } => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_RESULT_COERCED.to_string(),
                message:
                    "tool_result JSON content coerced to string for OpenAI function_call_output"
                        .to_string(),
            });
            Ok(stable_json_string(&canonicalize_json(value)))
        }
        ToolResultContent::Parts { parts } => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_RESULT_COERCED.to_string(),
                message: "tool_result parts content coerced to newline-delimited string for OpenAI function_call_output".to_string(),
            });

            let mut lines = Vec::new();
            for part in parts {
                match part {
                    ContentPart::Text { text } => lines.push(text.clone()),
                    _ => {
                        return Err(protocol_error(
                            Some(&req.model.model_id),
                            "tool_result parts content for OpenAI must contain only text parts",
                        ));
                    }
                }
            }
            Ok(lines.join("\n"))
        }
    }
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

fn stable_json_string(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn is_strict_compatible_schema(schema: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return false;
    };

    if obj.contains_key("anyOf") || obj.contains_key("oneOf") || obj.contains_key("allOf") {
        return false;
    }

    let is_object_schema = is_object_type(obj.get("type"));
    if !is_object_schema {
        if let Some(items) = obj.get("items") {
            return is_strict_compatible_schema(items);
        }
        return true;
    }

    match obj.get("additionalProperties") {
        Some(Value::Bool(false)) => {}
        _ => return false,
    }

    let properties = obj
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let required = obj
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if properties.len() != required.len() {
        return false;
    }

    for key in properties.keys() {
        let present = required
            .iter()
            .filter_map(Value::as_str)
            .any(|required_key| required_key == key);
        if !present {
            return false;
        }
    }

    properties.values().all(is_strict_compatible_schema)
}

fn is_object_type(type_value: Option<&Value>) -> bool {
    match type_value {
        Some(Value::String(value)) => value == "object",
        Some(Value::Array(values)) => values.iter().any(|entry| entry == "object"),
        _ => false,
    }
}

fn decode_output_item(
    item: &Value,
    content: &mut Vec<ContentPart>,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<(), ProviderError> {
    let item_obj = item
        .as_object()
        .ok_or_else(|| protocol_error(None, "output item must be an object"))?;
    let item_type = item_obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(None, "output item missing type"))?;

    match item_type {
        "message" => decode_output_message(item_obj, content, warnings),
        "function_call" => decode_output_tool_call(item_obj, content, warnings),
        "reasoning" => Ok(()),
        "refusal" => {
            if let Some(text) = extract_refusal_text(item_obj) {
                content.push(ContentPart::Text { text });
                warnings.push(RuntimeWarning {
                    code: WARN_MODEL_REFUSAL.to_string(),
                    message: "OpenAI refusal content mapped to canonical text".to_string(),
                });
            }
            Ok(())
        }
        other => Err(protocol_error(
            None,
            format!("unsupported output item type: {other}"),
        )),
    }
}

fn decode_output_message(
    item_obj: &Map<String, Value>,
    content: &mut Vec<ContentPart>,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<(), ProviderError> {
    let parts = item_obj
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for part in parts {
        let Some(part_obj) = part.as_object() else {
            return Err(protocol_error(
                None,
                "output message content part must be an object",
            ));
        };

        let Some(part_type) = part_obj.get("type").and_then(Value::as_str) else {
            return Err(protocol_error(
                None,
                "output message content part missing type",
            ));
        };

        match part_type {
            "output_text" => {
                let text = part_obj
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    content.push(ContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            "reasoning" => {}
            "refusal" => {
                if let Some(text) = extract_refusal_text(part_obj) {
                    content.push(ContentPart::Text { text });
                    warnings.push(RuntimeWarning {
                        code: WARN_MODEL_REFUSAL.to_string(),
                        message: "OpenAI refusal content mapped to canonical text".to_string(),
                    });
                }
            }
            other => {
                return Err(protocol_error(
                    None,
                    format!("unsupported output message content part type: {other}"),
                ));
            }
        }
    }

    Ok(())
}

fn decode_output_tool_call(
    item_obj: &Map<String, Value>,
    content: &mut Vec<ContentPart>,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<(), ProviderError> {
    let call_id = item_obj
        .get("call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(None, "function_call output item missing call_id"))?;
    let name = item_obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(None, "function_call output item missing name"))?;
    let arguments = item_obj
        .get("arguments")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error(None, "function_call output item missing arguments"))?;

    let arguments_json = match serde_json::from_str::<Value>(arguments) {
        Ok(value) => value,
        Err(_) => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_ARGUMENTS_INVALID_JSON.to_string(),
                message: "OpenAI tool call arguments were not valid JSON; stored raw string"
                    .to_string(),
            });
            Value::String(arguments.to_string())
        }
    };

    content.push(ContentPart::ToolCall {
        tool_call: ToolCall {
            id: call_id.to_string(),
            name: name.to_string(),
            arguments_json,
        },
    });

    Ok(())
}

fn extract_refusal_text(obj: &Map<String, Value>) -> Option<String> {
    if let Some(text) = obj.get("text").and_then(Value::as_str) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }

    if let Some(text) = obj.get("refusal").and_then(Value::as_str) {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }

    None
}

fn decode_structured_output(
    requested_response_format: &ResponseFormat,
    content: &[ContentPart],
    warnings: &mut Vec<RuntimeWarning>,
    model: Option<&str>,
) -> Option<Value> {
    if matches!(requested_response_format, ResponseFormat::Text) {
        return None;
    }

    let joined_text = content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if joined_text.trim().is_empty() {
        return None;
    }

    match serde_json::from_str::<Value>(&joined_text) {
        Ok(parsed) => match requested_response_format {
            ResponseFormat::JsonObject => {
                if parsed.is_object() {
                    Some(parsed)
                } else {
                    warnings.push(RuntimeWarning {
                        code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                        message: "structured output was valid JSON but not an object".to_string(),
                    });
                    None
                }
            }
            ResponseFormat::JsonSchema { .. } => Some(parsed),
            ResponseFormat::Text => None,
        },
        Err(error) => {
            warnings.push(RuntimeWarning {
                code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                message: format!(
                    "failed to parse structured output JSON{}: {error}",
                    format_model_context(model)
                ),
            });
            None
        }
    }
}

fn decode_usage(usage: Option<&Value>, warnings: &mut Vec<RuntimeWarning>) -> Usage {
    let Some(usage_obj) = usage.and_then(Value::as_object) else {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_MISSING.to_string(),
            message: "openai response missing usage details".to_string(),
        });
        return Usage::default();
    };

    let input_tokens = usage_obj.get("input_tokens").and_then(Value::as_u64);
    let output_tokens = usage_obj.get("output_tokens").and_then(Value::as_u64);
    let total_tokens = usage_obj.get("total_tokens").and_then(Value::as_u64);
    let cached_input_tokens = usage_obj
        .get("input_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);

    Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens,
        total_tokens,
    }
}

fn map_finish_reason(
    status: &str,
    incomplete_reason: Option<&str>,
    content: &[ContentPart],
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<FinishReason, ProviderError> {
    match status {
        "completed" => {
            if should_finish_with_tool_calls(content) {
                Ok(FinishReason::ToolCalls)
            } else {
                Ok(FinishReason::Stop)
            }
        }
        "incomplete" => match incomplete_reason {
            Some("max_output_tokens") | Some("max_tokens") => {
                warnings.push(RuntimeWarning {
                    code: WARN_OPENAI_INCOMPLETE_MAX_OUTPUT_TOKENS.to_string(),
                    message: "openai response incomplete because max_output_tokens was reached"
                        .to_string(),
                });
                Ok(FinishReason::Length)
            }
            Some("content_filter") => {
                warnings.push(RuntimeWarning {
                    code: WARN_OPENAI_INCOMPLETE_CONTENT_FILTER.to_string(),
                    message: "openai response incomplete because of content filtering".to_string(),
                });
                Ok(FinishReason::ContentFilter)
            }
            Some(reason) => {
                warnings.push(RuntimeWarning {
                    code: WARN_OPENAI_INCOMPLETE_UNKNOWN_REASON.to_string(),
                    message: format!("openai response incomplete for reason: {reason}"),
                });
                Ok(FinishReason::Other)
            }
            None => {
                warnings.push(RuntimeWarning {
                    code: WARN_OPENAI_INCOMPLETE_MISSING_REASON.to_string(),
                    message: "openai response incomplete with no reason".to_string(),
                });
                Ok(FinishReason::Other)
            }
        },
        "cancelled" => Err(protocol_error(None, "openai response status is cancelled")),
        "failed" => Err(protocol_error(None, "openai response status is failed")),
        "in_progress" | "queued" => Err(protocol_error(
            None,
            format!("openai response status is non-terminal: {status}"),
        )),
        other => Err(protocol_error(
            None,
            format!("unknown openai response status: {other}"),
        )),
    }
}

fn parse_openai_error_value(root: &Map<String, Value>) -> Option<OpenAiErrorEnvelope> {
    let error = root.get("error")?.as_object()?;
    let message = value_to_string(error.get("message"))
        .unwrap_or_else(|| "openai response reported an error".to_string());

    Some(OpenAiErrorEnvelope {
        message,
        code: value_to_string(error.get("code")),
        error_type: value_to_string(error.get("type")),
        param: value_to_string(error.get("param")),
    })
}

fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(Value::Number(number)) => Some(number.to_string()),
        Some(Value::Bool(flag)) => Some(flag.to_string()),
        _ => None,
    }
}

fn should_finish_with_tool_calls(content: &[ContentPart]) -> bool {
    let mut saw_tool_call = false;
    let mut saw_text_after_tool_call = false;

    for part in content {
        match part {
            ContentPart::ToolCall { .. } => saw_tool_call = true,
            ContentPart::Text { text } if saw_tool_call && !text.trim().is_empty() => {
                saw_text_after_tool_call = true;
            }
            _ => {}
        }
    }

    saw_tool_call && !saw_text_after_tool_call
}

fn protocol_error(model: Option<&str>, message: impl Into<String>) -> ProviderError {
    ProviderError::Protocol {
        provider: ProviderId::Openai,
        model: model.map(str::to_string),
        request_id: None,
        message: message.into(),
    }
}

fn format_model_context(model: Option<&str>) -> String {
    match model {
        Some(model) => format!(" for model {model}"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests;
