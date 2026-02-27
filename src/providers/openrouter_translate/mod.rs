use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::core::error::ProviderError;
use crate::core::types::{
    AssistantOutput, ContentPart, FinishReason, Message, MessageRole, ModelInfo, ProviderId,
    ProviderRequest, ProviderResponse, ResponseFormat, RuntimeWarning, ToolCall, ToolChoice,
    ToolDefinition, ToolResult, ToolResultContent, Usage,
};
use crate::providers::translator_contract::ProviderTranslator;

const WARN_BOTH_TEMPERATURE_AND_TOP_P_SET: &str = "both_temperature_and_top_p_set";
const WARN_TOOL_ARGUMENTS_INVALID_JSON: &str = "tool_arguments_invalid_json";
const WARN_USAGE_MISSING: &str = "usage_missing";
const WARN_USAGE_PARTIAL: &str = "usage_partial";
const WARN_STRUCTURED_OUTPUT_PARSE_FAILED: &str = "structured_output_parse_failed";
const WARN_UNKNOWN_FINISH_REASON: &str = "unknown_finish_reason";
const WARN_EMPTY_OUTPUT: &str = "empty_output";
const WARN_TOOL_RESULT_COERCED: &str = "tool_result_coerced";
const WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED: &str =
    "tool_result_raw_provider_content_ignored";

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct OpenRouterTranslateOptions {
    pub fallback_models: Vec<String>,
    pub provider_preferences: Option<Value>,
    pub plugins: Vec<Value>,
    pub parallel_tool_calls: Option<bool>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub logit_bias: Option<Value>,
    pub logprobs: Option<bool>,
    pub top_logprobs: Option<u8>,
    pub reasoning: Option<Value>,
    pub seed: Option<i64>,
    pub user: Option<String>,
    pub session_id: Option<String>,
    pub trace: Option<Value>,
    pub route: Option<String>,
    pub max_tokens: Option<u32>,
    pub modalities: Option<Vec<String>>,
    pub image_config: Option<Value>,
    pub debug: Option<Value>,
    pub stream_options: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenRouterEncodedRequest {
    pub body: Value,
    pub warnings: Vec<RuntimeWarning>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenRouterDecodeEnvelope {
    pub body: Value,
    pub requested_response_format: ResponseFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenRouterErrorEnvelope {
    pub code: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct OpenRouterTranslator {
    options: OpenRouterTranslateOptions,
}

impl OpenRouterTranslator {
    pub(crate) fn new(options: OpenRouterTranslateOptions) -> Self {
        Self { options }
    }
}

impl ProviderTranslator for OpenRouterTranslator {
    type RequestPayload = OpenRouterEncodedRequest;
    type ResponsePayload = OpenRouterDecodeEnvelope;

    fn encode_request(&self, req: &ProviderRequest) -> Result<Self::RequestPayload, ProviderError> {
        encode_openrouter_request(req, &self.options)
    }

    fn decode_response(
        &self,
        payload: &Self::ResponsePayload,
    ) -> Result<ProviderResponse, ProviderError> {
        decode_openrouter_response(payload)
    }
}

pub(crate) fn encode_openrouter_request(
    req: &ProviderRequest,
    options: &OpenRouterTranslateOptions,
) -> Result<OpenRouterEncodedRequest, ProviderError> {
    validate_provider_hint(req)?;
    validate_model_id(req)?;
    validate_stop(req)?;
    validate_metadata(req)?;
    validate_sampling_controls(req)?;

    let mut warnings = Vec::new();
    if req.temperature.is_some() && req.top_p.is_some() {
        warnings.push(RuntimeWarning {
            code: WARN_BOTH_TEMPERATURE_AND_TOP_P_SET.to_string(),
            message: "OpenRouter recommends setting temperature or top_p, but not both".to_string(),
        });
    }

    validate_options(options, &req.model.model_id)?;
    let tools = map_tools(req)?;
    let tool_choice = map_tool_choice(req, !tools.is_empty())?;
    let messages = map_messages(req, !tools.is_empty(), &mut warnings)?;
    let response_format = map_response_format(req)?;

    if messages.is_empty() {
        return Err(protocol_error(Some(&req.model.model_id), "empty messages"));
    }

    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        Value::String(req.model.model_id.clone()),
    );
    body.insert("messages".to_string(), Value::Array(messages));
    body.insert("stream".to_string(), Value::Bool(false));

    if !options.fallback_models.is_empty() {
        let mut models = Vec::with_capacity(1 + options.fallback_models.len());
        models.push(Value::String(req.model.model_id.clone()));
        for fallback in &options.fallback_models {
            models.push(Value::String(fallback.clone()));
        }
        body.remove("model");
        body.insert("models".to_string(), Value::Array(models));
    }

    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }

    if let Some(tool_choice) = tool_choice {
        body.insert("tool_choice".to_string(), tool_choice);
    }

    if let Some(response_format) = response_format {
        body.insert("response_format".to_string(), response_format);
    }

    if let Some(temperature) = req.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(top_p) = req.top_p {
        body.insert("top_p".to_string(), json!(top_p));
    }

    if let Some(frequency_penalty) = options.frequency_penalty {
        body.insert("frequency_penalty".to_string(), json!(frequency_penalty));
    }

    if let Some(presence_penalty) = options.presence_penalty {
        body.insert("presence_penalty".to_string(), json!(presence_penalty));
    }

    if let Some(logit_bias) = &options.logit_bias {
        body.insert("logit_bias".to_string(), logit_bias.clone());
    }

    if let Some(logprobs) = options.logprobs {
        body.insert("logprobs".to_string(), Value::Bool(logprobs));
    }

    if let Some(top_logprobs) = options.top_logprobs {
        body.insert("top_logprobs".to_string(), json!(top_logprobs));
    }

    if let Some(reasoning) = &options.reasoning {
        body.insert("reasoning".to_string(), reasoning.clone());
    }

    if let Some(max_output_tokens) = req.max_output_tokens {
        body.insert(
            "max_completion_tokens".to_string(),
            json!(max_output_tokens),
        );
    }

    if let Some(max_tokens) = options.max_tokens {
        body.insert("max_tokens".to_string(), json!(max_tokens));
    }

    if let Some(seed) = options.seed {
        body.insert("seed".to_string(), json!(seed));
    }

    if !req.stop.is_empty() {
        body.insert("stop".to_string(), json!(req.stop));
    }

    if !req.metadata.is_empty() {
        body.insert("metadata".to_string(), json!(req.metadata));
    }

    if let Some(value) = options.parallel_tool_calls {
        body.insert("parallel_tool_calls".to_string(), Value::Bool(value));
    }

    if let Some(provider) = &options.provider_preferences {
        body.insert("provider".to_string(), provider.clone());
    }

    if let Some(user) = &options.user {
        body.insert("user".to_string(), Value::String(user.clone()));
    }

    if let Some(session_id) = &options.session_id {
        body.insert("session_id".to_string(), Value::String(session_id.clone()));
    }

    if let Some(trace) = &options.trace {
        body.insert("trace".to_string(), trace.clone());
    }

    if let Some(route) = &options.route {
        body.insert("route".to_string(), Value::String(route.clone()));
    }

    if let Some(modalities) = &options.modalities {
        body.insert("modalities".to_string(), json!(modalities));
    }

    if let Some(image_config) = &options.image_config {
        body.insert("image_config".to_string(), image_config.clone());
    }

    if let Some(debug) = &options.debug {
        body.insert("debug".to_string(), debug.clone());
    }

    if let Some(stream_options) = &options.stream_options {
        body.insert("stream_options".to_string(), stream_options.clone());
    }

    if !options.plugins.is_empty() {
        body.insert("plugins".to_string(), Value::Array(options.plugins.clone()));
    }

    Ok(OpenRouterEncodedRequest {
        body: Value::Object(body),
        warnings,
    })
}

pub(crate) fn decode_openrouter_response(
    payload: &OpenRouterDecodeEnvelope,
) -> Result<ProviderResponse, ProviderError> {
    let root = payload
        .body
        .as_object()
        .ok_or_else(|| protocol_error(None, "openrouter response payload must be a JSON object"))?;

    if let Some(error) = parse_openrouter_error_value(root) {
        return Err(protocol_error(
            None,
            format_openrouter_error_message(&error),
        ));
    }

    let model = root
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("<unknown-model>")
        .to_string();

    let choices = root
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error(Some(&model), "openrouter response missing choices array"))?;

    if choices.is_empty() {
        return Err(protocol_error(
            Some(&model),
            "openrouter response choices array must not be empty",
        ));
    }

    let choice = choices[0].as_object().ok_or_else(|| {
        protocol_error(
            Some(&model),
            "openrouter response choices[0] must be a JSON object",
        )
    })?;

    if let Some(choice_error) = choice.get("error") {
        return Err(protocol_error(
            Some(&model),
            format!(
                "openrouter response choice contained error: {}",
                stable_json_string(choice_error)
            ),
        ));
    }

    let finish_reason_raw = choice.get("finish_reason").and_then(Value::as_str);
    if finish_reason_raw == Some("error") {
        return Err(protocol_error(
            Some(&model),
            "openrouter response finish_reason was error",
        ));
    }

    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            protocol_error(Some(&model), "openrouter response missing choice message")
        })?;

    if let Some(role) = message.get("role").and_then(Value::as_str)
        && role != "assistant"
    {
        return Err(protocol_error(
            Some(&model),
            format!("openrouter response message role must be assistant, got {role}"),
        ));
    }

    let mut warnings = Vec::new();
    let mut content = Vec::new();
    let mut text_blocks = Vec::new();

    decode_message_content(message.get("content"), &mut content, &mut text_blocks)?;
    decode_refusal(message.get("refusal"), &mut content, &mut text_blocks)?;
    decode_tool_calls(
        message.get("tool_calls"),
        &mut content,
        &mut warnings,
        &model,
    )?;

    if content.is_empty() {
        warnings.push(RuntimeWarning {
            code: WARN_EMPTY_OUTPUT.to_string(),
            message: "openrouter response contained no decodable output content".to_string(),
        });
    }

    let finish_reason = map_finish_reason(finish_reason_raw, &model, &mut warnings);
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
        provider: ProviderId::Openrouter,
        model,
        raw_provider_response: None,
        finish_reason,
        warnings,
    })
}

pub(crate) fn parse_openrouter_error_envelope(body: &str) -> Option<OpenRouterErrorEnvelope> {
    let payload = serde_json::from_str::<Value>(body).ok()?;
    let root = payload.as_object()?;
    parse_openrouter_error_value(root)
}

pub(crate) fn format_openrouter_error_message(envelope: &OpenRouterErrorEnvelope) -> String {
    match envelope.code {
        Some(code) => format!("openrouter error: {} [code={code}]", envelope.message),
        None => format!("openrouter error: {}", envelope.message),
    }
}

pub(crate) fn decode_openrouter_models_list(
    payload: &Value,
) -> Result<Vec<ModelInfo>, ProviderError> {
    let root = payload
        .as_object()
        .ok_or_else(|| protocol_error(None, "openrouter models payload must be a JSON object"))?;
    let data = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| protocol_error(None, "openrouter models payload missing data array"))?;

    let mut discovered = Vec::new();
    let mut seen = BTreeSet::new();

    for (index, item) in data.iter().enumerate() {
        let model_obj = item.as_object().ok_or_else(|| {
            protocol_error(
                None,
                format!("openrouter models payload contains non-object entry at index {index}"),
            )
        })?;

        let model_id = model_obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                protocol_error(
                    None,
                    format!("openrouter models payload entry missing id at index {index}"),
                )
            })?
            .trim()
            .to_string();

        if model_id.is_empty() {
            return Err(protocol_error(
                None,
                format!("openrouter models payload entry has empty id at index {index}"),
            ));
        }

        if !seen.insert(model_id.clone()) {
            continue;
        }

        let context_window = model_obj
            .get("top_provider")
            .and_then(Value::as_object)
            .and_then(|top| top.get("context_length"))
            .or_else(|| model_obj.get("context_length"))
            .and_then(number_to_u32);

        let max_output_tokens = model_obj
            .get("top_provider")
            .and_then(Value::as_object)
            .and_then(|top| top.get("max_completion_tokens"))
            .and_then(number_to_u32);

        let (supports_tools, supports_structured_output) = decode_model_capabilities(model_obj);

        discovered.push(ModelInfo {
            provider: ProviderId::Openrouter,
            model_id,
            display_name: model_obj
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            context_window,
            max_output_tokens,
            supports_tools,
            supports_structured_output,
        });
    }

    Ok(discovered)
}

fn validate_provider_hint(req: &ProviderRequest) -> Result<(), ProviderError> {
    if let Some(provider_hint) = &req.model.provider_hint {
        if *provider_hint != ProviderId::Openrouter {
            return Err(protocol_error(
                Some(&req.model.model_id),
                format!("provider_hint must be Openrouter, got {provider_hint:?}"),
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
    if req.stop.len() > 4 {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "stop supports at most 4 entries",
        ));
    }

    Ok(())
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

    if req.max_output_tokens == Some(0) {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "max_output_tokens must be at least 1",
        ));
    }

    Ok(())
}

fn validate_options(
    options: &OpenRouterTranslateOptions,
    model_id: &str,
) -> Result<(), ProviderError> {
    for fallback in &options.fallback_models {
        if fallback.trim().is_empty() {
            return Err(protocol_error(
                Some(model_id),
                "fallback_models must not include empty model ids",
            ));
        }
    }

    if let Some(provider) = &options.provider_preferences
        && !provider.is_object()
    {
        return Err(protocol_error(
            Some(model_id),
            "provider preferences must be a JSON object",
        ));
    }

    for (index, plugin) in options.plugins.iter().enumerate() {
        if !plugin.is_object() {
            return Err(protocol_error(
                Some(model_id),
                format!("plugin at index {index} must be a JSON object"),
            ));
        }
    }

    if let Some(frequency_penalty) = options.frequency_penalty
        && !(-2.0..=2.0).contains(&frequency_penalty)
    {
        return Err(protocol_error(
            Some(model_id),
            format!("frequency_penalty must be in [-2.0, 2.0], got {frequency_penalty}"),
        ));
    }

    if let Some(presence_penalty) = options.presence_penalty
        && !(-2.0..=2.0).contains(&presence_penalty)
    {
        return Err(protocol_error(
            Some(model_id),
            format!("presence_penalty must be in [-2.0, 2.0], got {presence_penalty}"),
        ));
    }

    if let Some(logit_bias) = &options.logit_bias {
        let Some(entries) = logit_bias.as_object() else {
            return Err(protocol_error(
                Some(model_id),
                "logit_bias must be a JSON object",
            ));
        };
        for (token, bias) in entries {
            if !bias.is_number() {
                return Err(protocol_error(
                    Some(model_id),
                    format!("logit_bias value for token '{token}' must be numeric"),
                ));
            }
        }
    }

    if let Some(top_logprobs) = options.top_logprobs
        && top_logprobs > 20
    {
        return Err(protocol_error(
            Some(model_id),
            format!("top_logprobs must be in [0, 20], got {top_logprobs}"),
        ));
    }

    if let Some(reasoning) = &options.reasoning
        && !reasoning.is_object()
    {
        return Err(protocol_error(
            Some(model_id),
            "reasoning must be a JSON object",
        ));
    }

    if let Some(user) = &options.user
        && user.trim().is_empty()
    {
        return Err(protocol_error(
            Some(model_id),
            "user must be non-empty when provided",
        ));
    }

    if let Some(session_id) = &options.session_id {
        if session_id.trim().is_empty() {
            return Err(protocol_error(
                Some(model_id),
                "session_id must be non-empty when provided",
            ));
        }
        if session_id.chars().count() > 128 {
            return Err(protocol_error(
                Some(model_id),
                "session_id must be 128 characters or fewer",
            ));
        }
    }

    if let Some(trace) = &options.trace
        && !trace.is_object()
    {
        return Err(protocol_error(
            Some(model_id),
            "trace must be a JSON object",
        ));
    }

    if let Some(route) = &options.route
        && route != "fallback"
        && route != "sort"
    {
        return Err(protocol_error(
            Some(model_id),
            "route must be 'fallback' or 'sort' when provided",
        ));
    }

    if options.max_tokens == Some(0) {
        return Err(protocol_error(
            Some(model_id),
            "max_tokens must be at least 1",
        ));
    }

    if let Some(modalities) = &options.modalities {
        if modalities.is_empty() {
            return Err(protocol_error(
                Some(model_id),
                "modalities must be non-empty when provided",
            ));
        }
        for modality in modalities {
            if modality != "text" {
                return Err(protocol_error(
                    Some(model_id),
                    format!(
                        "modalities only supports 'text' in non-streaming canonical mode; got '{modality}'"
                    ),
                ));
            }
        }
    }

    if options.image_config.is_some() {
        return Err(protocol_error(
            Some(model_id),
            "image_config is unsupported in non-streaming canonical mode",
        ));
    }

    if options.debug.is_some() {
        return Err(protocol_error(
            Some(model_id),
            "debug is unsupported in non-streaming canonical mode",
        ));
    }

    if options.stream_options.is_some() {
        return Err(protocol_error(
            Some(model_id),
            "stream_options is unsupported in non-streaming canonical mode",
        ));
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

fn map_tool_definition(tool: &ToolDefinition, model_id: &str) -> Result<Value, ProviderError> {
    if !is_valid_tool_name(&tool.name) {
        return Err(protocol_error(
            Some(model_id),
            format!(
                "tool '{}' name must match ^[A-Za-z0-9_-]{{1,64}}$",
                tool.name
            ),
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

    let mut function = Map::new();
    function.insert("name".to_string(), Value::String(tool.name.clone()));
    if let Some(description) = &tool.description {
        function.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    function.insert("parameters".to_string(), tool.parameters_schema.clone());

    Ok(json!({
        "type": "function",
        "function": Value::Object(function),
    }))
}

fn map_tool_choice(req: &ProviderRequest, has_tools: bool) -> Result<Option<Value>, ProviderError> {
    if !has_tools {
        return match &req.tool_choice {
            ToolChoice::Required => Err(protocol_error(
                Some(&req.model.model_id),
                "tool_choice required requires at least one tool definition",
            )),
            ToolChoice::Specific { .. } => Err(protocol_error(
                Some(&req.model.model_id),
                "tool_choice specific requires at least one tool definition",
            )),
            ToolChoice::None | ToolChoice::Auto => Ok(None),
        };
    }

    match &req.tool_choice {
        ToolChoice::None => Ok(Some(Value::String("none".to_string()))),
        ToolChoice::Auto => Ok(Some(Value::String("auto".to_string()))),
        ToolChoice::Required => Ok(Some(Value::String("required".to_string()))),
        ToolChoice::Specific { name } => {
            if name.trim().is_empty() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "tool_choice specific requires non-empty name",
                ));
            }

            if !req.tools.iter().any(|tool| tool.name == *name) {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    format!("tool_choice specific references unknown tool: {name}"),
                ));
            }

            Ok(Some(json!({
                "type": "function",
                "function": { "name": name }
            })))
        }
    }
}

fn map_response_format(req: &ProviderRequest) -> Result<Option<Value>, ProviderError> {
    match &req.response_format {
        ResponseFormat::Text => Ok(None),
        ResponseFormat::JsonObject => Ok(Some(json!({ "type": "json_object" }))),
        ResponseFormat::JsonSchema { name, schema } => {
            if name.trim().is_empty() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "json_schema response format requires non-empty name",
                ));
            }

            if name.chars().count() > 64 {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "json_schema name exceeds 64 characters",
                ));
            }

            if !schema.is_object() {
                return Err(protocol_error(
                    Some(&req.model.model_id),
                    "json_schema schema must be a JSON object",
                ));
            }

            Ok(Some(json!({
                "type": "json_schema",
                "json_schema": {
                    "name": name,
                    "schema": schema,
                    "strict": true
                }
            })))
        }
    }
}

fn map_messages(
    req: &ProviderRequest,
    has_tools: bool,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Vec<Value>, ProviderError> {
    let mut messages = Vec::new();
    let mut saw_tool_role = false;

    for message in &req.messages {
        messages.push(map_message(message, &req.model.model_id, warnings)?);
        if message.role == MessageRole::Tool {
            saw_tool_role = true;
        }
    }

    if saw_tool_role && !has_tools {
        return Err(protocol_error(
            Some(&req.model.model_id),
            "tool messages require at least one tool definition",
        ));
    }

    Ok(messages)
}

fn map_message(
    message: &Message,
    model_id: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Value, ProviderError> {
    match message.role {
        MessageRole::System => map_string_message("system", &message.content, model_id),
        MessageRole::User => map_string_message("user", &message.content, model_id),
        MessageRole::Assistant => map_assistant_message(&message.content, model_id),
        MessageRole::Tool => map_tool_message(&message.content, model_id, warnings),
    }
}

fn map_string_message(
    role: &str,
    content: &[ContentPart],
    model_id: &str,
) -> Result<Value, ProviderError> {
    let text = join_text_parts(content, model_id, role, true)?;
    Ok(json!({
        "role": role,
        "content": text,
    }))
}

fn map_assistant_message(content: &[ContentPart], model_id: &str) -> Result<Value, ProviderError> {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for part in content {
        match part {
            ContentPart::Text { text } => text_parts.push(text.clone()),
            ContentPart::ToolCall { tool_call } => {
                if tool_call.id.trim().is_empty() {
                    return Err(protocol_error(
                        Some(model_id),
                        "assistant tool_call id must be non-empty",
                    ));
                }
                if tool_call.name.trim().is_empty() {
                    return Err(protocol_error(
                        Some(model_id),
                        "assistant tool_call name must be non-empty",
                    ));
                }
                if !is_valid_tool_name(&tool_call.name) {
                    return Err(protocol_error(
                        Some(model_id),
                        format!(
                            "assistant tool_call '{}' name must match ^[A-Za-z0-9_-]{{1,64}}$",
                            tool_call.name
                        ),
                    ));
                }

                let arguments = stable_json_string(&canonicalize_json(&tool_call.arguments_json));
                tool_calls.push(json!({
                    "id": tool_call.id,
                    "type": "function",
                    "function": {
                        "name": tool_call.name,
                        "arguments": arguments,
                    }
                }));
            }
            ContentPart::ToolResult { .. } => {
                return Err(protocol_error(
                    Some(model_id),
                    "tool_result content is only valid for tool role messages",
                ));
            }
        }
    }

    if text_parts.is_empty() && tool_calls.is_empty() {
        return Err(protocol_error(
            Some(model_id),
            "assistant messages must contain text or tool_calls",
        ));
    }

    let mut payload = Map::new();
    payload.insert("role".to_string(), Value::String("assistant".to_string()));

    if text_parts.is_empty() {
        payload.insert("content".to_string(), Value::Null);
    } else {
        payload.insert("content".to_string(), Value::String(text_parts.join("\n")));
    }

    if !tool_calls.is_empty() {
        payload.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    Ok(Value::Object(payload))
}

fn map_tool_message(
    content: &[ContentPart],
    model_id: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Value, ProviderError> {
    if content.len() != 1 {
        return Err(protocol_error(
            Some(model_id),
            "tool role messages must contain exactly one tool_result part",
        ));
    }

    let tool_result = match &content[0] {
        ContentPart::ToolResult { tool_result } => tool_result,
        _ => {
            return Err(protocol_error(
                Some(model_id),
                "tool role messages must contain tool_result content",
            ));
        }
    };

    if tool_result.tool_call_id.trim().is_empty() {
        return Err(protocol_error(
            Some(model_id),
            "tool_result tool_call_id must be non-empty",
        ));
    }

    let output = coerce_tool_result_output(tool_result, model_id, warnings)?;

    Ok(json!({
        "role": "tool",
        "tool_call_id": tool_result.tool_call_id,
        "content": output,
    }))
}

fn coerce_tool_result_output(
    tool_result: &ToolResult,
    model_id: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<String, ProviderError> {
    if let Some(raw_provider_content) = &tool_result.raw_provider_content {
        if let Some(raw_text) = raw_provider_content.as_str() {
            return Ok(raw_text.to_string());
        }

        warnings.push(RuntimeWarning {
            code: WARN_TOOL_RESULT_RAW_PROVIDER_CONTENT_IGNORED.to_string(),
            message:
                "tool_result raw_provider_content ignored for OpenRouter because it is not a string"
                    .to_string(),
        });
    }

    match &tool_result.content {
        ToolResultContent::Text { text } => Ok(text.clone()),
        ToolResultContent::Json { value } => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_RESULT_COERCED.to_string(),
                message: "tool_result JSON content coerced to string for OpenRouter tool message"
                    .to_string(),
            });
            Ok(stable_json_string(&canonicalize_json(value)))
        }
        ToolResultContent::Parts { parts } => {
            warnings.push(RuntimeWarning {
                code: WARN_TOOL_RESULT_COERCED.to_string(),
                message: "tool_result parts content coerced to newline-delimited string for OpenRouter tool message".to_string(),
            });
            join_text_parts(parts, model_id, "tool_result", false)
        }
    }
}

fn join_text_parts(
    content: &[ContentPart],
    model_id: &str,
    context: &str,
    allow_empty: bool,
) -> Result<String, ProviderError> {
    let mut parts = Vec::new();

    for part in content {
        match part {
            ContentPart::Text { text } => parts.push(text.clone()),
            _ => {
                return Err(protocol_error(
                    Some(model_id),
                    format!("{context} content must contain only text parts"),
                ));
            }
        }
    }

    if !allow_empty && parts.is_empty() {
        return Err(protocol_error(
            Some(model_id),
            format!("{context} content must contain at least one text part"),
        ));
    }

    Ok(parts.join("\n"))
}

fn is_valid_tool_name(name: &str) -> bool {
    if name.is_empty() || name.chars().count() > 64 {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn decode_message_content(
    value: Option<&Value>,
    content: &mut Vec<ContentPart>,
    text_blocks: &mut Vec<String>,
) -> Result<(), ProviderError> {
    let Some(value) = value else {
        return Ok(());
    };

    match value {
        Value::Null => Ok(()),
        Value::String(text) => {
            if !text.is_empty() {
                text_blocks.push(text.clone());
                content.push(ContentPart::Text { text: text.clone() });
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                let item_obj = item.as_object().ok_or_else(|| {
                    protocol_error(None, "assistant content array item must be an object")
                })?;
                let item_type = item_obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if item_type == "text" {
                    let text = item_obj
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| protocol_error(None, "text content item missing text"))?;
                    text_blocks.push(text.to_string());
                    content.push(ContentPart::Text {
                        text: text.to_string(),
                    });
                } else {
                    return Err(protocol_error(
                        None,
                        format!(
                            "assistant content item type '{item_type}' is unsupported in canonical text mode"
                        ),
                    ));
                }
            }
            Ok(())
        }
        _ => Err(protocol_error(
            None,
            "assistant content must be string, array, or null",
        )),
    }
}

fn decode_refusal(
    refusal_value: Option<&Value>,
    content: &mut Vec<ContentPart>,
    text_blocks: &mut Vec<String>,
) -> Result<(), ProviderError> {
    let Some(refusal_value) = refusal_value else {
        return Ok(());
    };

    if refusal_value.is_null() {
        return Ok(());
    }

    let refusal = refusal_value
        .as_str()
        .ok_or_else(|| protocol_error(None, "assistant refusal must be a string or null"))?;

    if refusal.is_empty() {
        return Ok(());
    }

    text_blocks.push(refusal.to_string());
    content.push(ContentPart::Text {
        text: refusal.to_string(),
    });
    Ok(())
}

fn decode_tool_calls(
    value: Option<&Value>,
    content: &mut Vec<ContentPart>,
    warnings: &mut Vec<RuntimeWarning>,
    model: &str,
) -> Result<(), ProviderError> {
    let Some(value) = value else {
        return Ok(());
    };

    let calls = value
        .as_array()
        .ok_or_else(|| protocol_error(Some(model), "tool_calls must be an array"))?;

    for call in calls {
        let call_obj = call
            .as_object()
            .ok_or_else(|| protocol_error(Some(model), "tool_call entry must be an object"))?;
        let id = call_obj
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error(Some(model), "tool_call missing id"))?;
        if id.trim().is_empty() {
            return Err(protocol_error(
                Some(model),
                "tool_call id must be non-empty",
            ));
        }

        let call_type = call_obj
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error(Some(model), "tool_call missing type"))?;
        if call_type != "function" {
            return Err(protocol_error(
                Some(model),
                format!("tool_call type must be function, got {call_type}"),
            ));
        }

        let function = call_obj
            .get("function")
            .and_then(Value::as_object)
            .ok_or_else(|| protocol_error(Some(model), "tool_call missing function object"))?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error(Some(model), "tool_call function missing name"))?;
        let args_raw = function
            .get("arguments")
            .and_then(Value::as_str)
            .ok_or_else(|| protocol_error(Some(model), "tool_call function missing arguments"))?;

        let arguments_json = match serde_json::from_str::<Value>(args_raw) {
            Ok(value) => value,
            Err(_) => {
                warnings.push(RuntimeWarning {
                    code: WARN_TOOL_ARGUMENTS_INVALID_JSON.to_string(),
                    message: format!(
                        "openrouter tool_call arguments were not valid JSON for call_id={id}"
                    ),
                });
                Value::String(args_raw.to_string())
            }
        };

        content.push(ContentPart::ToolCall {
            tool_call: ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments_json,
            },
        });
    }

    Ok(())
}

fn decode_usage(
    usage_value: Option<&Value>,
    model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Result<Usage, ProviderError> {
    let Some(usage_value) = usage_value else {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_MISSING.to_string(),
            message: "openrouter response usage was missing".to_string(),
        });
        return Ok(Usage::default());
    };

    if usage_value.is_null() {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_MISSING.to_string(),
            message: "openrouter response usage was null".to_string(),
        });
        return Ok(Usage::default());
    }

    let usage_obj = usage_value
        .as_object()
        .ok_or_else(|| protocol_error(Some(model), "usage must be an object or null"))?;

    let input_tokens = usage_obj.get("prompt_tokens").and_then(number_to_u64);
    let output_tokens = usage_obj.get("completion_tokens").and_then(number_to_u64);
    let total_tokens = usage_obj.get("total_tokens").and_then(number_to_u64);

    let cached_input_tokens = usage_obj
        .get("prompt_tokens_details")
        .and_then(Value::as_object)
        .and_then(|details| details.get("cached_tokens"))
        .and_then(number_to_u64);

    let usage = Usage {
        input_tokens,
        output_tokens,
        cached_input_tokens,
        total_tokens,
    };

    if usage.input_tokens.is_none() || usage.output_tokens.is_none() || usage.total_tokens.is_none()
    {
        warnings.push(RuntimeWarning {
            code: WARN_USAGE_PARTIAL.to_string(),
            message: "openrouter response usage was partial".to_string(),
        });
    }

    Ok(usage)
}

fn decode_structured_output(
    response_format: &ResponseFormat,
    text_blocks: &[String],
    _model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> Option<Value> {
    if matches!(response_format, ResponseFormat::Text) {
        return None;
    }

    if text_blocks.is_empty() {
        return None;
    }

    let joined = text_blocks.join("\n");
    match serde_json::from_str::<Value>(&joined) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(RuntimeWarning {
                code: WARN_STRUCTURED_OUTPUT_PARSE_FAILED.to_string(),
                message: format!("failed to parse structured output JSON: {error}"),
            });
            None
        }
    }
}

fn map_finish_reason(
    finish_reason: Option<&str>,
    _model: &str,
    warnings: &mut Vec<RuntimeWarning>,
) -> FinishReason {
    match finish_reason {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some("content_filter") => FinishReason::ContentFilter,
        Some("error") => FinishReason::Error,
        Some(other) => {
            warnings.push(RuntimeWarning {
                code: WARN_UNKNOWN_FINISH_REASON.to_string(),
                message: format!("openrouter finish_reason '{other}' mapped to Other"),
            });
            FinishReason::Other
        }
        None => FinishReason::Other,
    }
}

fn parse_openrouter_error_value(root: &Map<String, Value>) -> Option<OpenRouterErrorEnvelope> {
    let error_obj = root.get("error")?.as_object()?;
    let message = error_obj
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)?;

    let code = error_obj
        .get("code")
        .and_then(number_to_u64)
        .and_then(|value| u16::try_from(value).ok());

    Some(OpenRouterErrorEnvelope { code, message })
}

fn decode_model_capabilities(model_obj: &Map<String, Value>) -> (bool, bool) {
    let Some(supported_parameters) = model_obj
        .get("supported_parameters")
        .and_then(Value::as_array)
    else {
        return (true, true);
    };

    let mut supports_tools = false;
    let mut supports_structured_output = false;

    for parameter in supported_parameters {
        let Some(param) = parameter.as_str() else {
            continue;
        };

        match param {
            "tools" => supports_tools = true,
            "response_format" | "structured_outputs" => supports_structured_output = true,
            _ => {}
        }
    }

    (supports_tools, supports_structured_output)
}

fn number_to_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        .or_else(|| {
            value.as_f64().and_then(|value| {
                if value.is_finite() && value >= 0.0 {
                    Some(value as u64)
                } else {
                    None
                }
            })
        })
}

fn number_to_u32(value: &Value) -> Option<u32> {
    number_to_u64(value).and_then(|value| u32::try_from(value).ok())
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

fn protocol_error(model: Option<&str>, message: impl Into<String>) -> ProviderError {
    ProviderError::Protocol {
        provider: ProviderId::Openrouter,
        model: model.map(str::to_string),
        request_id: None,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
