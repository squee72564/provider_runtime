use async_trait::async_trait;
use serde_json::Value;

use crate::core::error::{ConfigError, ProviderError};
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, DiscoveryOptions, ModelInfo, ProviderCapabilities, ProviderId, ProviderRequest,
    ProviderResponse,
};
use crate::providers::openrouter_translate::{
    OpenRouterDecodeEnvelope, OpenRouterTranslateOptions, OpenRouterTranslator,
    decode_openrouter_models_list, format_openrouter_error_message,
    parse_openrouter_error_envelope,
};
use crate::providers::translator_contract::ProviderTranslator;
use crate::transport::http::{HttpTransport, RetryPolicy};

const OPENROUTER_DEFAULT_BASE_URL: &str = "https://openrouter.ai";
const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
const OPENROUTER_API_KEY_METADATA: &str = "openrouter.api_key";

const TRANSPORT_AUTH_BEARER_TOKEN_KEY: &str = "transport.auth.bearer_token";
const TRANSPORT_HEADER_HTTP_REFERER: &str = "transport.header.http-referer";
const TRANSPORT_HEADER_X_TITLE: &str = "transport.header.x-title";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct OpenRouterAdapterOptions {
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
    pub http_referer: Option<String>,
    pub x_title: Option<String>,
}

impl OpenRouterAdapterOptions {
    fn validate(&self) -> Result<(), ConfigError> {
        for model in &self.fallback_models {
            if model.trim().is_empty() {
                return Err(Self::invalid_config(
                    "fallback_models must not include empty model ids",
                ));
            }
        }

        if let Some(provider_preferences) = &self.provider_preferences
            && !provider_preferences.is_object()
        {
            return Err(Self::invalid_config(
                "provider_preferences must be a JSON object",
            ));
        }

        for (index, plugin) in self.plugins.iter().enumerate() {
            if !plugin.is_object() {
                return Err(Self::invalid_config(format!(
                    "plugins[{index}] must be a JSON object"
                )));
            }
        }

        if let Some(frequency_penalty) = self.frequency_penalty
            && !(-2.0..=2.0).contains(&frequency_penalty)
        {
            return Err(Self::invalid_config(format!(
                "frequency_penalty must be in [-2.0, 2.0], got {frequency_penalty}"
            )));
        }

        if let Some(presence_penalty) = self.presence_penalty
            && !(-2.0..=2.0).contains(&presence_penalty)
        {
            return Err(Self::invalid_config(format!(
                "presence_penalty must be in [-2.0, 2.0], got {presence_penalty}"
            )));
        }

        if let Some(top_logprobs) = self.top_logprobs
            && top_logprobs > 20
        {
            return Err(Self::invalid_config(format!(
                "top_logprobs must be in [0, 20], got {top_logprobs}"
            )));
        }

        if let Some(logit_bias) = &self.logit_bias {
            let Some(map) = logit_bias.as_object() else {
                return Err(Self::invalid_config("logit_bias must be a JSON object"));
            };
            for (token, bias) in map {
                if !bias.is_number() {
                    return Err(Self::invalid_config(format!(
                        "logit_bias value for token '{token}' must be numeric"
                    )));
                }
            }
        }

        if let Some(reasoning) = &self.reasoning
            && !reasoning.is_object()
        {
            return Err(Self::invalid_config("reasoning must be a JSON object"));
        }

        if let Some(trace) = &self.trace
            && !trace.is_object()
        {
            return Err(Self::invalid_config("trace must be a JSON object"));
        }

        if let Some(user) = &self.user
            && user.trim().is_empty()
        {
            return Err(Self::invalid_config("user must be non-empty when provided"));
        }

        if let Some(session_id) = &self.session_id {
            if session_id.trim().is_empty() {
                return Err(Self::invalid_config(
                    "session_id must be non-empty when provided",
                ));
            }
            if session_id.chars().count() > 128 {
                return Err(Self::invalid_config(
                    "session_id must be 128 characters or fewer",
                ));
            }
        }

        if let Some(route) = &self.route
            && route != "fallback"
            && route != "sort"
        {
            return Err(Self::invalid_config(
                "route must be 'fallback' or 'sort' when provided",
            ));
        }

        if self.max_tokens == Some(0) {
            return Err(Self::invalid_config("max_tokens must be at least 1"));
        }

        if let Some(modalities) = &self.modalities {
            if modalities.is_empty() {
                return Err(Self::invalid_config(
                    "modalities must be non-empty when provided",
                ));
            }
            for modality in modalities {
                if modality != "text" {
                    return Err(Self::invalid_config(format!(
                        "modalities only supports 'text' in non-streaming canonical mode; got '{modality}'"
                    )));
                }
            }
        }

        if self.image_config.is_some() {
            return Err(Self::invalid_config(
                "image_config is unsupported in non-streaming canonical mode",
            ));
        }

        if self.debug.is_some() {
            return Err(Self::invalid_config(
                "debug is unsupported in non-streaming canonical mode",
            ));
        }

        if self.stream_options.is_some() {
            return Err(Self::invalid_config(
                "stream_options is unsupported in non-streaming canonical mode",
            ));
        }

        if let Some(http_referer) = &self.http_referer
            && http_referer.trim().is_empty()
        {
            return Err(Self::invalid_config(
                "http_referer must be non-empty when provided",
            ));
        }

        if let Some(x_title) = &self.x_title
            && x_title.trim().is_empty()
        {
            return Err(Self::invalid_config(
                "x_title must be non-empty when provided",
            ));
        }

        Ok(())
    }

    pub(crate) fn to_translate_options(&self) -> OpenRouterTranslateOptions {
        OpenRouterTranslateOptions {
            fallback_models: self.fallback_models.clone(),
            provider_preferences: self.provider_preferences.clone(),
            plugins: self.plugins.clone(),
            parallel_tool_calls: self.parallel_tool_calls,
            frequency_penalty: self.frequency_penalty,
            presence_penalty: self.presence_penalty,
            logit_bias: self.logit_bias.clone(),
            logprobs: self.logprobs,
            top_logprobs: self.top_logprobs,
            reasoning: self.reasoning.clone(),
            seed: self.seed,
            user: self.user.clone(),
            session_id: self.session_id.clone(),
            trace: self.trace.clone(),
            route: self.route.clone(),
            max_tokens: self.max_tokens,
            modalities: self.modalities.clone(),
            image_config: self.image_config.clone(),
            debug: self.debug.clone(),
            stream_options: self.stream_options.clone(),
        }
    }

    fn invalid_config(reason: impl Into<String>) -> ConfigError {
        ConfigError::InvalidProviderConfig {
            provider: ProviderId::Openrouter,
            reason: reason.into(),
        }
    }
}

pub struct OpenRouterAdapter {
    transport: HttpTransport,
    translator: OpenRouterTranslator,
    base_url: String,
    api_key: Option<String>,
    options: OpenRouterAdapterOptions,
}

impl OpenRouterAdapter {
    pub fn new(api_key: Option<String>) -> Result<Self, ConfigError> {
        Self::with_base_url_and_options(api_key, OPENROUTER_DEFAULT_BASE_URL, Default::default())
    }

    pub fn with_base_url(
        api_key: Option<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        Self::with_base_url_and_options(api_key, base_url, Default::default())
    }

    pub fn with_base_url_and_options(
        api_key: Option<String>,
        base_url: impl Into<String>,
        options: OpenRouterAdapterOptions,
    ) -> Result<Self, ConfigError> {
        options.validate()?;
        let transport = HttpTransport::new(30_000, RetryPolicy::default())?;
        Ok(Self::with_transport(api_key, base_url, options, transport))
    }

    pub(crate) fn with_transport(
        api_key: Option<String>,
        base_url: impl Into<String>,
        options: OpenRouterAdapterOptions,
        transport: HttpTransport,
    ) -> Self {
        let translator = OpenRouterTranslator::new(options.to_translate_options());

        Self {
            transport,
            translator,
            base_url: normalize_base_url(base_url),
            api_key: sanitize_api_key(api_key),
            options,
        }
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/api/v1/chat/completions", self.base_url)
    }

    fn models_url(&self) -> String {
        format!("{}/api/v1/models", self.base_url)
    }

    fn resolve_api_key(&self, ctx: &AdapterContext) -> Option<String> {
        if let Some(key) = self.api_key.as_ref().cloned() {
            return Some(key);
        }

        if let Some(key) = ctx.metadata.get(OPENROUTER_API_KEY_METADATA)
            && !key.trim().is_empty()
        {
            return Some(key.clone());
        }

        std::env::var(OPENROUTER_API_KEY_ENV)
            .ok()
            .and_then(|value| sanitize_api_key(Some(value)))
    }

    fn missing_api_key_error(model: Option<&str>) -> ProviderError {
        ProviderError::Protocol {
            provider: ProviderId::Openrouter,
            model: model.map(str::to_string),
            request_id: None,
            message: format!(
                "missing OpenRouter API key; set {OPENROUTER_API_KEY_METADATA} metadata or {OPENROUTER_API_KEY_ENV} env var"
            ),
        }
    }

    fn attach_transport_context(
        &self,
        ctx: &AdapterContext,
        api_key: Option<String>,
    ) -> AdapterContext {
        let mut request_ctx = ctx.clone();

        if let Some(api_key) = api_key {
            request_ctx
                .metadata
                .insert(TRANSPORT_AUTH_BEARER_TOKEN_KEY.to_string(), api_key);
        }

        if let Some(http_referer) = self.options.http_referer.as_ref() {
            request_ctx.metadata.insert(
                TRANSPORT_HEADER_HTTP_REFERER.to_string(),
                http_referer.clone(),
            );
        }

        if let Some(x_title) = self.options.x_title.as_ref() {
            request_ctx
                .metadata
                .insert(TRANSPORT_HEADER_X_TITLE.to_string(), x_title.clone());
        }

        request_ctx
    }

    fn normalize_transport_error(
        error: ProviderError,
        requested_model: Option<&str>,
    ) -> ProviderError {
        match error {
            ProviderError::Status {
                status_code,
                request_id,
                message,
                model,
                ..
            } => {
                let model = requested_model.map(str::to_string).or(model);

                if let Some(envelope) = parse_openrouter_error_envelope(&message) {
                    let message = format_openrouter_error_message(&envelope);
                    if status_code == 401 || status_code == 403 {
                        return ProviderError::CredentialsRejected {
                            provider: ProviderId::Openrouter,
                            request_id,
                            message,
                        };
                    }

                    return ProviderError::Status {
                        provider: ProviderId::Openrouter,
                        model,
                        status_code,
                        request_id,
                        message,
                    };
                }

                if status_code == 401 || status_code == 403 {
                    return ProviderError::CredentialsRejected {
                        provider: ProviderId::Openrouter,
                        request_id,
                        message,
                    };
                }

                ProviderError::Status {
                    provider: ProviderId::Openrouter,
                    model,
                    status_code,
                    request_id,
                    message,
                }
            }
            other => other,
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenRouterAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Openrouter
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_tools: true,
            supports_structured_output: true,
            supports_thinking: true,
            supports_remote_discovery: true,
        }
    }

    async fn run(
        &self,
        req: &ProviderRequest,
        ctx: &AdapterContext,
    ) -> Result<ProviderResponse, ProviderError> {
        let api_key = self
            .resolve_api_key(ctx)
            .ok_or_else(|| Self::missing_api_key_error(Some(&req.model.model_id)))?;

        let encoded = self.translator.encode_request(req)?;
        let request_ctx = self.attach_transport_context(ctx, Some(api_key));

        let response_body: Value = self
            .transport
            .post_json(
                ProviderId::Openrouter,
                Some(req.model.model_id.as_str()),
                &self.chat_completions_url(),
                &encoded.body,
                &request_ctx,
            )
            .await
            .map_err(|error| Self::normalize_transport_error(error, Some(&req.model.model_id)))?;

        let envelope = OpenRouterDecodeEnvelope {
            body: response_body,
            requested_response_format: req.response_format.clone(),
        };

        let mut decoded = self.translator.decode_response(&envelope)?;
        if !encoded.warnings.is_empty() {
            let mut warnings = encoded.warnings;
            warnings.extend(decoded.warnings);
            decoded.warnings = warnings;
        }

        Ok(decoded)
    }

    async fn discover_models(
        &self,
        _opts: &DiscoveryOptions,
        ctx: &AdapterContext,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        let request_ctx = self.attach_transport_context(ctx, self.resolve_api_key(ctx));

        let payload: Value = self
            .transport
            .get_json(
                ProviderId::Openrouter,
                None,
                &self.models_url(),
                &request_ctx,
            )
            .await
            .map_err(|error| Self::normalize_transport_error(error, None))?;

        decode_openrouter_models_list(&payload)
    }
}

fn normalize_base_url(base_url: impl Into<String>) -> String {
    let value = base_url.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return OPENROUTER_DEFAULT_BASE_URL.to_string();
    }

    trimmed.trim_end_matches('/').to_string()
}

fn sanitize_api_key(api_key: Option<String>) -> Option<String> {
    api_key.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[cfg(test)]
mod tests;
