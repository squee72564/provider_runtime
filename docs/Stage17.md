# Stage 17 - Anthropic Adapter

Goal:
Implement Anthropic `ProviderAdapter` with capability declaration.

Files:
- [src/providers/anthropic/mod.rs](src/providers/anthropic/mod.rs)

Public API Surface:
- `struct AnthropicAdapter`
- `impl ProviderAdapter for AnthropicAdapter`

Internal Responsibilities:
- Capability declaration.
- Run/discover via transport + auth + translator composition.
- Deterministic auth error handling.
- Adapter orchestration must not perform ad hoc canonical<->provider field mapping.
- All protocol mapping must go through the shared crate-private translator contract.

Unit Tests:
- `test_anthropic_adapter_capabilities`
- `test_anthropic_adapter_missing_key_error`
- `test_anthropic_adapter_uses_translator_boundary`

Acceptance Criteria:
- Canonical output only.
- No pricing or registry internal usage.
- Adapter responsibilities are limited to orchestration (auth/transport/capabilities/error envelope handling).
- `cargo check --lib` passes.

Depends On:
- Stage 16
- Stage 10
