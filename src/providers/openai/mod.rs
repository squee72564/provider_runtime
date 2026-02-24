use async_trait::async_trait;
use serde_json::Value;

use crate::core::error::{ConfigError, ProviderError};
use crate::core::traits::ProviderAdapter;
use crate::core::types::{
    AdapterContext, DiscoveryOptions, ModelInfo, ProviderCapabilities, ProviderId, ProviderRequest,
    ProviderResponse,
};
use crate::providers::openai_translate::{OpenAiDecodeEnvelope, OpenAiTranslator};
use crate::providers::translator_contract::ProviderTranslator;
use crate::transport::http::{HttpTransport, RetryPolicy};

const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com";
const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
const OPENAI_API_KEY_METADATA: &str = "openai.api_key";
const TRANSPORT_AUTH_BEARER_TOKEN_KEY: &str = "transport.auth.bearer_token";

pub struct OpenAiAdapter {
    transport: HttpTransport,
    translator: OpenAiTranslator,
    base_url: String,
    api_key: Option<String>,
}

impl OpenAiAdapter {
    pub fn new(api_key: Option<String>) -> Result<Self, ConfigError> {
        Self::with_base_url(api_key, OPENAI_DEFAULT_BASE_URL)
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
            translator: OpenAiTranslator,
            base_url: normalize_base_url(base_url),
            api_key: sanitize_api_key(api_key),
        }
    }

    fn responses_url(&self) -> String {
        format!("{}/v1/responses", self.base_url)
    }

    fn resolve_api_key(&self, ctx: &AdapterContext) -> Option<String> {
        if let Some(key) = self.api_key.as_ref().cloned() {
            return Some(key);
        }

        if let Some(key) = ctx.metadata.get(OPENAI_API_KEY_METADATA) {
            if !key.trim().is_empty() {
                return Some(key.clone());
            }
        }

        std::env::var(OPENAI_API_KEY_ENV)
            .ok()
            .and_then(|value| sanitize_api_key(Some(value)))
    }

    fn missing_api_key_error(model: Option<&str>) -> ProviderError {
        ProviderError::Protocol {
            provider: ProviderId::Openai,
            model: model.map(str::to_string),
            request_id: None,
            message: format!(
                "missing OpenAI API key; set {OPENAI_API_KEY_METADATA} metadata or {OPENAI_API_KEY_ENV} env var"
            ),
        }
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Openai
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            supports_tools: true,
            supports_structured_output: true,
            supports_thinking: false,
            supports_remote_discovery: false,
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

        let mut request_ctx = ctx.clone();
        request_ctx
            .metadata
            .insert(TRANSPORT_AUTH_BEARER_TOKEN_KEY.to_string(), api_key);

        let response_body: Value = self
            .transport
            .post_json(
                ProviderId::Openai,
                Some(req.model.model_id.as_str()),
                &self.responses_url(),
                &encoded.body,
                &request_ctx,
            )
            .await?;

        let envelope = OpenAiDecodeEnvelope {
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
        _ctx: &AdapterContext,
    ) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }
}

fn normalize_base_url(base_url: impl Into<String>) -> String {
    let value = base_url.into();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return OPENAI_DEFAULT_BASE_URL.to_string();
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
