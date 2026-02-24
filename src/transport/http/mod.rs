use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::core::error::{ConfigError, ProviderError};
use crate::core::types::{AdapterContext, ProviderId};

const AUTH_BEARER_TOKEN_KEY: &str = "transport.auth.bearer_token";
const CUSTOM_HEADER_PREFIX: &str = "transport.header.";
const REQUEST_ID_HEADER_KEY: &str = "transport.request_id_header";
const DEFAULT_REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub retryable_status_codes: Vec<u16>,
}

impl RetryPolicy {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_attempts == 0 {
            return Err(ConfigError::InvalidRetryPolicy {
                reason: "max_attempts must be >= 1".to_string(),
            });
        }
        if self.max_backoff_ms < self.initial_backoff_ms {
            return Err(ConfigError::InvalidRetryPolicy {
                reason: "max_backoff_ms must be >= initial_backoff_ms".to_string(),
            });
        }
        if let Some(status) = self
            .retryable_status_codes
            .iter()
            .copied()
            .find(|status| !(100..=599).contains(status))
        {
            return Err(ConfigError::InvalidRetryPolicy {
                reason: format!("retryable status code must be in 100..=599: {status}"),
            });
        }
        Ok(())
    }

    fn should_retry_status(&self, status_code: u16) -> bool {
        self.retryable_status_codes.contains(&status_code)
    }

    fn backoff_duration_for_retry(&self, retry_index: u32) -> Duration {
        let shift = retry_index.min(63);
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let backoff_ms = self
            .initial_backoff_ms
            .saturating_mul(multiplier)
            .min(self.max_backoff_ms);
        Duration::from_millis(backoff_ms)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 2_000,
            retryable_status_codes: vec![408, 429, 500, 502, 503, 504],
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    retry_policy: RetryPolicy,
    timeout_ms: u64,
}

impl HttpTransport {
    pub fn new(timeout_ms: u64, retry_policy: RetryPolicy) -> Result<Self, ConfigError> {
        Self::validate_timeout(timeout_ms)?;
        retry_policy.validate()?;

        Ok(Self {
            client: reqwest::Client::new(),
            retry_policy,
            timeout_ms,
        })
    }

    pub fn with_client(
        client: reqwest::Client,
        timeout_ms: u64,
        retry_policy: RetryPolicy,
    ) -> Result<Self, ConfigError> {
        Self::validate_timeout(timeout_ms)?;
        retry_policy.validate()?;

        Ok(Self {
            client,
            retry_policy,
            timeout_ms,
        })
    }

    pub async fn get_json<TResp>(
        &self,
        provider: ProviderId,
        model: Option<&str>,
        url: &str,
        ctx: &AdapterContext,
    ) -> Result<TResp, ProviderError>
    where
        TResp: DeserializeOwned,
    {
        self.execute_json_request(provider, model, Method::GET, url, None, ctx)
            .await
    }

    pub async fn post_json<TReq, TResp>(
        &self,
        provider: ProviderId,
        model: Option<&str>,
        url: &str,
        body: &TReq,
        ctx: &AdapterContext,
    ) -> Result<TResp, ProviderError>
    where
        TReq: Serialize + ?Sized,
        TResp: DeserializeOwned,
    {
        let payload = serde_json::to_vec(body).map_err(|error| ProviderError::Serialization {
            provider: provider.clone(),
            model: model.map(str::to_string),
            request_id: None,
            message: error.to_string(),
        })?;

        self.execute_json_request(provider, model, Method::POST, url, Some(payload), ctx)
            .await
    }

    async fn execute_json_request<TResp>(
        &self,
        provider: ProviderId,
        model: Option<&str>,
        method: Method,
        url: &str,
        body: Option<Vec<u8>>,
        ctx: &AdapterContext,
    ) -> Result<TResp, ProviderError>
    where
        TResp: DeserializeOwned,
    {
        let header_config = self.build_header_config(&provider, model, ctx)?;
        let model_owned = model.map(str::to_string);

        let mut attempt: u32 = 0;
        loop {
            attempt += 1;

            let mut request_builder = self
                .client
                .request(method.clone(), url)
                .timeout(Duration::from_millis(self.timeout_ms))
                .headers(header_config.headers.clone());

            if let Some(payload) = &body {
                request_builder = request_builder
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(payload.clone());
            }

            match request_builder.send().await {
                Ok(response) => {
                    let status_code = response.status().as_u16();
                    let request_id =
                        extract_request_id(response.headers(), &header_config.request_id_header);

                    if !response.status().is_success() {
                        let status_error = self
                            .build_status_error(
                                &provider,
                                model_owned.as_deref(),
                                status_code,
                                request_id,
                                response,
                            )
                            .await;

                        if attempt < self.retry_policy.max_attempts
                            && self.retry_policy.should_retry_status(status_code)
                        {
                            self.sleep_before_retry(attempt).await;
                            continue;
                        }

                        return Err(status_error);
                    }

                    let parsed = response.json::<TResp>().await.map_err(|error| {
                        ProviderError::Serialization {
                            provider: provider.clone(),
                            model: model_owned.clone(),
                            request_id,
                            message: error.to_string(),
                        }
                    })?;

                    return Ok(parsed);
                }
                Err(error) => {
                    let transport_error = ProviderError::Transport {
                        provider: provider.clone(),
                        request_id: None,
                        message: error.to_string(),
                    };

                    if attempt < self.retry_policy.max_attempts && is_retryable_transport(&error) {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }

                    return Err(transport_error);
                }
            }
        }
    }

    async fn build_status_error(
        &self,
        provider: &ProviderId,
        model: Option<&str>,
        status_code: u16,
        request_id: Option<String>,
        response: Response,
    ) -> ProviderError {
        let message = match response.text().await {
            Ok(body) if !body.trim().is_empty() => body,
            Ok(_) => format!("http status {status_code}"),
            Err(error) => {
                format!("http status {status_code}; failed to read response body: {error}")
            }
        };

        ProviderError::Status {
            provider: provider.clone(),
            model: model.map(str::to_string),
            status_code,
            request_id,
            message,
        }
    }

    fn build_header_config(
        &self,
        provider: &ProviderId,
        model: Option<&str>,
        ctx: &AdapterContext,
    ) -> Result<HeaderConfig, ProviderError> {
        let request_id_header = match ctx.metadata.get(REQUEST_ID_HEADER_KEY) {
            Some(value) => parse_header_name(value, provider, model)?,
            None => HeaderName::from_static(DEFAULT_REQUEST_ID_HEADER),
        };

        let mut headers = HeaderMap::new();
        if let Some(token) = ctx.metadata.get(AUTH_BEARER_TOKEN_KEY) {
            let auth_value =
                HeaderValue::from_str(&format!("Bearer {token}")).map_err(|error| {
                    ProviderError::Protocol {
                        provider: provider.clone(),
                        model: model.map(str::to_string),
                        request_id: None,
                        message: format!("invalid bearer token header value: {error}"),
                    }
                })?;
            headers.insert(AUTHORIZATION, auth_value);
        }

        for (key, value) in &ctx.metadata {
            if let Some(raw_name) = key.strip_prefix(CUSTOM_HEADER_PREFIX) {
                let header_name = parse_header_name(raw_name, provider, model)?;
                let header_value =
                    HeaderValue::from_str(value).map_err(|error| ProviderError::Protocol {
                        provider: provider.clone(),
                        model: model.map(str::to_string),
                        request_id: None,
                        message: format!("invalid header value for {raw_name}: {error}"),
                    })?;
                headers.insert(header_name, header_value);
            }
        }

        Ok(HeaderConfig {
            headers,
            request_id_header,
        })
    }

    fn validate_timeout(timeout_ms: u64) -> Result<(), ConfigError> {
        if timeout_ms == 0 {
            return Err(ConfigError::InvalidTimeout { timeout_ms });
        }
        Ok(())
    }

    async fn sleep_before_retry(&self, attempt: u32) {
        let retry_index = attempt.saturating_sub(1);
        let backoff = self.retry_policy.backoff_duration_for_retry(retry_index);
        tokio::time::sleep(backoff).await;
    }
}

struct HeaderConfig {
    headers: HeaderMap,
    request_id_header: HeaderName,
}

fn parse_header_name(
    value: &str,
    provider: &ProviderId,
    model: Option<&str>,
) -> Result<HeaderName, ProviderError> {
    HeaderName::from_bytes(value.as_bytes()).map_err(|error| ProviderError::Protocol {
        provider: provider.clone(),
        model: model.map(str::to_string),
        request_id: None,
        message: format!("invalid header name: {value}: {error}"),
    })
}

fn extract_request_id(headers: &HeaderMap, request_id_header: &HeaderName) -> Option<String> {
    headers
        .get(request_id_header)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn is_retryable_transport(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

#[cfg(test)]
mod tests;
