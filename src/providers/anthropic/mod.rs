use async_trait::async_trait;
use serde_json::Value;

use crate::core::error::{ConfigError, ProviderError};
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, DiscoveryOptions, ModelInfo, ProviderCapabilities, ProviderId, ProviderRequest,
    ProviderResponse,
};
use crate::providers::anthropic_translate::{
    AnthropicDecodeEnvelope, AnthropicTranslator, decode_anthropic_models_list,
    format_anthropic_error_message, parse_anthropic_error_envelope,
};
use crate::providers::translator_contract::ProviderTranslator;
use crate::transport::http::{HttpTransport, RetryPolicy};

const ANTHROPIC_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const ANTHROPIC_API_KEY_METADATA: &str = "anthropic.api_key";
const ANTHROPIC_VERSION: &str = "2023-06-01";

const TRANSPORT_HEADER_API_KEY: &str = "transport.header.x-api-key";
const TRANSPORT_HEADER_ANTHROPIC_VERSION: &str = "transport.header.anthropic-version";
const TRANSPORT_REQUEST_ID_HEADER: &str = "transport.request_id_header";

pub struct AnthropicAdapter {
    transport: HttpTransport,
    translator: AnthropicTranslator,
    base_url: String,
    api_key: Option<String>,
}

impl AnthropicAdapter {
    pub fn new(api_key: Option<String>) -> Result<Self, ConfigError> {
        Self::with_base_url(api_key, ANTHROPIC_DEFAULT_BASE_URL)
    }

    pub fn with_base_url(
        api_key: Option<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, ConfigError> {
        let transport = HttpTransport::new(30_000, RetryPolicy::default())?;
        Ok(Self::with_transport(api_key, base_url, transport))
    }

    pub(crate) fn with_transport(
        api_key: Option<String>,
        base_url: impl Into<String>,
        transport: HttpTransport,
    ) -> Self {
        Self {
            transport,
            translator: AnthropicTranslator,
            base_url: normalize_base_url(base_url),
            api_key: sanitize_api_key(api_key),
        }
    }

    fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url)
    }

    fn models_url(&self) -> String {
        format!("{}/v1/models", self.base_url)
    }

    fn resolve_api_key(&self, ctx: &AdapterContext) -> Option<String> {
        let env_api_key = std::env::var(ANTHROPIC_API_KEY_ENV).ok();
        self.resolve_api_key_with_env(ctx, env_api_key)
    }

    fn resolve_api_key_with_env(
        &self,
        ctx: &AdapterContext,
        env_api_key: Option<String>,
    ) -> Option<String> {
        if let Some(key) = self.api_key.as_ref().cloned() {
            return Some(key);
        }

        if let Some(key) = ctx.metadata.get(ANTHROPIC_API_KEY_METADATA)
            && !key.trim().is_empty()
        {
            return Some(key.clone());
        }

        env_api_key.and_then(|value| sanitize_api_key(Some(value)))
    }

    fn missing_api_key_error(model: Option<&str>) -> ProviderError {
        ProviderError::Protocol {
            provider: ProviderId::Anthropic,
            model: model.map(str::to_string),
            request_id: None,
            message: format!(
                "missing Anthropic API key; set {ANTHROPIC_API_KEY_METADATA} metadata or {ANTHROPIC_API_KEY_ENV} env var"
            ),
        }
    }

    fn attach_transport_headers(ctx: &AdapterContext, api_key: String) -> AdapterContext {
        let mut request_ctx = ctx.clone();
        request_ctx
            .metadata
            .insert(TRANSPORT_HEADER_API_KEY.to_string(), api_key);
        request_ctx.metadata.insert(
            TRANSPORT_HEADER_ANTHROPIC_VERSION.to_string(),
            ANTHROPIC_VERSION.to_string(),
        );
        request_ctx.metadata.insert(
            TRANSPORT_REQUEST_ID_HEADER.to_string(),
            "request-id".to_string(),
        );
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
                let mut request_id = request_id;
                let model = requested_model.map(str::to_string).or(model);

                if let Some(envelope) = parse_anthropic_error_envelope(&message) {
                    if request_id.is_none() {
                        request_id = envelope.request_id.clone();
                    }
                    let message = format_anthropic_error_message(&envelope);

                    if status_code == 401 {
                        return ProviderError::CredentialsRejected {
                            provider: ProviderId::Anthropic,
                            request_id,
                            message,
                        };
                    }

                    return ProviderError::Status {
                        provider: ProviderId::Anthropic,
                        model,
                        status_code,
                        request_id,
                        message,
                    };
                }

                if status_code == 401 {
                    return ProviderError::CredentialsRejected {
                        provider: ProviderId::Anthropic,
                        request_id,
                        message,
                    };
                }

                ProviderError::Status {
                    provider: ProviderId::Anthropic,
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
impl ProviderAdapter for AnthropicAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Anthropic
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
        let request_ctx = Self::attach_transport_headers(ctx, api_key);

        let response_body: Value = self
            .transport
            .post_json(
                ProviderId::Anthropic,
                Some(req.model.model_id.as_str()),
                &self.messages_url(),
                &encoded.body,
                &request_ctx,
            )
            .await
            .map_err(|error| Self::normalize_transport_error(error, Some(&req.model.model_id)))?;

        let envelope = AnthropicDecodeEnvelope {
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
        let api_key = self
            .resolve_api_key(ctx)
            .ok_or_else(|| Self::missing_api_key_error(None))?;

        let request_ctx = Self::attach_transport_headers(ctx, api_key);

        let payload: Value = self
            .transport
            .get_json(
                ProviderId::Anthropic,
                None,
                &self.models_url(),
                &request_ctx,
            )
            .await
            .map_err(|error| Self::normalize_transport_error(error, None))?;

        decode_anthropic_models_list(&payload, &self.capabilities())
    }
}

fn normalize_base_url(base_url: impl Into<String>) -> String {
    let value = base_url.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return ANTHROPIC_DEFAULT_BASE_URL.to_string();
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
