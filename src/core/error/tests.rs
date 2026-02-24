use super::*;
use crate::core::types::ProviderId;

#[test]
fn test_runtime_error_display_messages() {
    let config_error = ConfigError::InvalidProviderConfig {
        provider: ProviderId::Openai,
        reason: "missing api key".to_string(),
    };
    assert_eq!(
        config_error.to_string(),
        "invalid provider config for Openai: missing api key"
    );

    let missing_credentials = RuntimeError::credential_missing(
        ProviderId::Openai,
        vec![
            "OPENAI_API_KEY".to_string(),
            "ALT_KEY".to_string(),
            "OPENAI_API_KEY".to_string(),
        ],
    );
    assert_eq!(
        missing_credentials.to_string(),
        "credential missing [provider=Openai, env_candidates=ALT_KEY, OPENAI_API_KEY]"
    );

    let routing_error = RoutingError::ModelNotFound {
        model: "gpt-5-mini".to_string(),
    };
    assert_eq!(
        routing_error.to_string(),
        "model route not found: gpt-5-mini"
    );

    let transport_error = RuntimeError::TransportError {
        provider: Some(ProviderId::Openrouter),
        model: Some("openrouter/auto".to_string()),
        request_id: Some("req_123".to_string()),
        message: "timeout".to_string(),
    };
    assert_eq!(
        transport_error.to_string(),
        "transport error [provider=Openrouter, model=openrouter/auto, request_id=req_123]: timeout"
    );

    let protocol_with_status = RuntimeError::ProviderProtocolError {
        provider: Some(ProviderId::Openai),
        model: Some("gpt-5-mini".to_string()),
        request_id: Some("req_abc".to_string()),
        status_code: Some(429),
        message: "rate limited".to_string(),
    };
    assert_eq!(
        protocol_with_status.to_string(),
        "provider protocol error [provider=Openai, model=gpt-5-mini, request_id=req_abc, status_code=429]: rate limited"
    );

    let protocol_without_status = RuntimeError::ProviderProtocolError {
        provider: None,
        model: None,
        request_id: None,
        status_code: None,
        message: "invalid payload".to_string(),
    };
    assert_eq!(
        protocol_without_status.to_string(),
        "provider protocol error: invalid payload"
    );

    let cost_error = RuntimeError::CostCalculationError {
        provider: Some(ProviderId::Anthropic),
        model: Some("claude-3-7-sonnet".to_string()),
        message: "no pricing entry".to_string(),
    };
    assert_eq!(
        cost_error.to_string(),
        "cost calculation error [provider=Anthropic, model=claude-3-7-sonnet]: no pricing entry"
    );
}

#[test]
fn test_missing_credential_error_contains_env_hints() {
    let error = RuntimeError::credential_missing(
        ProviderId::Anthropic,
        vec![
            "ZZZ_KEY".to_string(),
            "AAA_KEY".to_string(),
            "".to_string(),
            "AAA_KEY".to_string(),
        ],
    );

    match &error {
        RuntimeError::CredentialMissing {
            provider,
            env_candidates,
        } => {
            assert_eq!(*provider, ProviderId::Anthropic);
            assert_eq!(
                env_candidates,
                &vec!["AAA_KEY".to_string(), "ZZZ_KEY".to_string()]
            );
        }
        _ => panic!("expected credential missing variant"),
    }

    let rendered = error.to_string();
    assert!(rendered.contains("provider=Anthropic"));
    assert!(rendered.contains("env_candidates=AAA_KEY, ZZZ_KEY"));
}

#[test]
fn test_provider_error_conversion_preserves_context() {
    let transport_runtime: RuntimeError = ProviderError::Transport {
        provider: ProviderId::Openrouter,
        request_id: Some("req_transport".to_string()),
        message: "connection reset".to_string(),
    }
    .into();
    assert_eq!(
        transport_runtime,
        RuntimeError::TransportError {
            provider: Some(ProviderId::Openrouter),
            model: None,
            request_id: Some("req_transport".to_string()),
            message: "connection reset".to_string(),
        }
    );

    let serialization_runtime: RuntimeError = ProviderError::Serialization {
        provider: ProviderId::Openai,
        model: Some("gpt-5-mini".to_string()),
        request_id: Some("req_serialization".to_string()),
        message: "decode failure".to_string(),
    }
    .into();
    assert_eq!(
        serialization_runtime,
        RuntimeError::SerializationError {
            provider: Some(ProviderId::Openai),
            model: Some("gpt-5-mini".to_string()),
            request_id: Some("req_serialization".to_string()),
            message: "decode failure".to_string(),
        }
    );

    let credentials_runtime: RuntimeError = ProviderError::CredentialsRejected {
        provider: ProviderId::Anthropic,
        request_id: Some("req_credentials".to_string()),
        message: "invalid key".to_string(),
    }
    .into();
    assert_eq!(
        credentials_runtime,
        RuntimeError::ProviderProtocolError {
            provider: Some(ProviderId::Anthropic),
            model: None,
            request_id: Some("req_credentials".to_string()),
            status_code: None,
            message: "invalid key".to_string(),
        }
    );

    let status_runtime: RuntimeError = ProviderError::Status {
        provider: ProviderId::Openai,
        model: Some("gpt-5-mini".to_string()),
        status_code: 429,
        request_id: Some("req_status".to_string()),
        message: "too many requests".to_string(),
    }
    .into();
    assert_eq!(
        status_runtime,
        RuntimeError::ProviderProtocolError {
            provider: Some(ProviderId::Openai),
            model: Some("gpt-5-mini".to_string()),
            request_id: Some("req_status".to_string()),
            status_code: Some(429),
            message: "too many requests".to_string(),
        }
    );

    let protocol_runtime: RuntimeError = ProviderError::Protocol {
        provider: ProviderId::Openrouter,
        model: Some("openrouter/auto".to_string()),
        request_id: Some("req_protocol".to_string()),
        message: "unexpected response shape".to_string(),
    }
    .into();
    assert_eq!(
        protocol_runtime,
        RuntimeError::ProviderProtocolError {
            provider: Some(ProviderId::Openrouter),
            model: Some("openrouter/auto".to_string()),
            request_id: Some("req_protocol".to_string()),
            status_code: None,
            message: "unexpected response shape".to_string(),
        }
    );
}
