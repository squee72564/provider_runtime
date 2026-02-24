use async_trait::async_trait;

use crate::core::error::{ProviderError, RuntimeError};
use crate::core::types::{
    AdapterContext, DiscoveryOptions, ModelInfo, ProviderCapabilities, ProviderId, ProviderRequest,
    ProviderResponse,
};

/// Provider adapter contract for translating canonical runtime requests to a
/// provider protocol and returning canonical responses.
///
/// This trait is an extension point only. v0 does not include loop orchestration,
/// session state, or provider protocol leakage in the public API.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    /// Stable provider identifier for routing and diagnostics.
    fn id(&self) -> ProviderId;

    /// Declares provider support flags used by runtime capability checks.
    fn capabilities(&self) -> ProviderCapabilities;

    /// Executes a single non-streaming canonical request.
    async fn run(
        &self,
        req: &ProviderRequest,
        ctx: &AdapterContext,
    ) -> Result<ProviderResponse, ProviderError>;

    /// Discovers provider models and maps results into canonical model records.
    async fn discover_models(
        &self,
        opts: &DiscoveryOptions,
        ctx: &AdapterContext,
    ) -> Result<Vec<ModelInfo>, ProviderError>;
}

/// Optional auth extension point for externally managed bearer token retrieval.
///
/// This is a hook only for future OAuth-compatible integrations. v0 does not
/// implement OAuth flows or token lifecycle management.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Returns an access token for the requested provider.
    async fn get_token(&self, provider: ProviderId) -> Result<String, RuntimeError>;
}

#[cfg(test)]
mod tests;
