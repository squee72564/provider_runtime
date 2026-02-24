use crate::core::types::ProviderId;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error("missing default provider configuration")]
    MissingDefaultProvider,
    #[error("invalid provider config for {provider:?}: {reason}")]
    InvalidProviderConfig {
        provider: ProviderId,
        reason: String,
    },
    #[error("invalid timeout: {timeout_ms} ms")]
    InvalidTimeout { timeout_ms: u64 },
    #[error("invalid retry policy: {reason}")]
    InvalidRetryPolicy { reason: String },
    #[error("invalid pricing config: {reason}")]
    InvalidPricingConfig { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RoutingError {
    #[error("provider not registered: {provider:?}")]
    ProviderNotRegistered { provider: ProviderId },
    #[error("model route not found: {model}")]
    ModelNotFound { model: String },
    #[error(
        "ambiguous model route for {model}: {candidates}",
        candidates = format_provider_candidates(.candidates)
    )]
    AmbiguousModelRoute {
        model: String,
        candidates: Vec<ProviderId>,
    },
    #[error(
        "provider hint mismatch for model {model}: hint={provider_hint:?} resolved={resolved:?}"
    )]
    ProviderHintMismatch {
        model: String,
        provider_hint: ProviderId,
        resolved: ProviderId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderError {
    #[error(
        "provider credentials rejected{context}: {message}",
        context = format_context(Some(.provider), None, .request_id.as_deref(), None)
    )]
    CredentialsRejected {
        provider: ProviderId,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "provider transport error{context}: {message}",
        context = format_context(Some(.provider), None, .request_id.as_deref(), None)
    )]
    Transport {
        provider: ProviderId,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "provider status error{context}: {message}",
        context = format_context(
            Some(.provider),
            .model.as_deref(),
            .request_id.as_deref(),
            Some(*.status_code)
        )
    )]
    Status {
        provider: ProviderId,
        model: Option<String>,
        status_code: u16,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "provider protocol error{context}: {message}",
        context = format_context(
            Some(.provider),
            .model.as_deref(),
            .request_id.as_deref(),
            None
        )
    )]
    Protocol {
        provider: ProviderId,
        model: Option<String>,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "provider serialization error{context}: {message}",
        context = format_context(
            Some(.provider),
            .model.as_deref(),
            .request_id.as_deref(),
            None
        )
    )]
    Serialization {
        provider: ProviderId,
        model: Option<String>,
        request_id: Option<String>,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    ConfigError(#[from] ConfigError),
    #[error(
        "credential missing [provider={provider:?}{env_candidates}]",
        env_candidates = format_env_candidates(.env_candidates)
    )]
    CredentialMissing {
        provider: ProviderId,
        env_candidates: Vec<String>,
    },
    #[error(transparent)]
    RoutingError(#[from] RoutingError),
    #[error("capability mismatch [provider={provider:?}, model={model}, capability={capability}]")]
    CapabilityMismatch {
        provider: ProviderId,
        model: String,
        capability: String,
    },
    #[error(
        "transport error{context}: {message}",
        context = format_context(.provider.as_ref(), .model.as_deref(), .request_id.as_deref(), None)
    )]
    TransportError {
        provider: Option<ProviderId>,
        model: Option<String>,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "provider protocol error{context}: {message}",
        context = format_context(
            .provider.as_ref(),
            .model.as_deref(),
            .request_id.as_deref(),
            *.status_code
        )
    )]
    ProviderProtocolError {
        provider: Option<ProviderId>,
        model: Option<String>,
        request_id: Option<String>,
        status_code: Option<u16>,
        message: String,
    },
    #[error(
        "serialization error{context}: {message}",
        context = format_context(.provider.as_ref(), .model.as_deref(), .request_id.as_deref(), None)
    )]
    SerializationError {
        provider: Option<ProviderId>,
        model: Option<String>,
        request_id: Option<String>,
        message: String,
    },
    #[error(
        "cost calculation error{context}: {message}",
        context = format_context(.provider.as_ref(), .model.as_deref(), None, None)
    )]
    CostCalculationError {
        provider: Option<ProviderId>,
        model: Option<String>,
        message: String,
    },
}

impl RuntimeError {
    pub fn credential_missing(provider: ProviderId, mut env_candidates: Vec<String>) -> Self {
        env_candidates.retain(|candidate| !candidate.is_empty());
        env_candidates.sort_unstable();
        env_candidates.dedup();

        Self::CredentialMissing {
            provider,
            env_candidates,
        }
    }
}

impl From<ProviderError> for RuntimeError {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::Transport {
                provider,
                request_id,
                message,
            } => Self::TransportError {
                provider: Some(provider),
                model: None,
                request_id,
                message,
            },
            ProviderError::Serialization {
                provider,
                model,
                request_id,
                message,
            } => Self::SerializationError {
                provider: Some(provider),
                model,
                request_id,
                message,
            },
            ProviderError::CredentialsRejected {
                provider,
                request_id,
                message,
            } => Self::ProviderProtocolError {
                provider: Some(provider),
                model: None,
                request_id,
                status_code: None,
                message,
            },
            ProviderError::Status {
                provider,
                model,
                status_code,
                request_id,
                message,
            } => Self::ProviderProtocolError {
                provider: Some(provider),
                model,
                request_id,
                status_code: Some(status_code),
                message,
            },
            ProviderError::Protocol {
                provider,
                model,
                request_id,
                message,
            } => Self::ProviderProtocolError {
                provider: Some(provider),
                model,
                request_id,
                status_code: None,
                message,
            },
        }
    }
}

fn format_provider_candidates(candidates: &[ProviderId]) -> String {
    let mut rendered: Vec<String> = candidates
        .iter()
        .map(|provider| format!("{provider:?}"))
        .collect();
    rendered.sort_unstable();
    rendered.join(", ")
}

fn format_env_candidates(env_candidates: &[String]) -> String {
    if env_candidates.is_empty() {
        String::new()
    } else {
        format!(", env_candidates={}", env_candidates.join(", "))
    }
}

fn format_context(
    provider: Option<&ProviderId>,
    model: Option<&str>,
    request_id: Option<&str>,
    status_code: Option<u16>,
) -> String {
    let mut context = Vec::new();

    if let Some(provider) = provider {
        context.push(format!("provider={provider:?}"));
    }
    if let Some(model) = model {
        context.push(format!("model={model}"));
    }
    if let Some(request_id) = request_id {
        context.push(format!("request_id={request_id}"));
    }
    if let Some(status_code) = status_code {
        context.push(format!("status_code={status_code}"));
    }

    if context.is_empty() {
        String::new()
    } else {
        format!(" [{}]", context.join(", "))
    }
}

#[cfg(test)]
mod tests;
