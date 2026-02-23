# Stage 17 - Anthropic Adapter

Goal:
Implement Anthropic `ProviderAdapter` with capability declaration.

Files:
- [src/providers/anthropic.rs](src/providers/anthropic.rs)

Public API Surface:
- `struct AnthropicAdapter`
- `impl ProviderAdapter for AnthropicAdapter`

Internal Responsibilities:
- Capability declaration.
- Run/discover via transport + translator.
- Deterministic auth error handling.

Unit Tests:
- `test_anthropic_adapter_capabilities`
- `test_anthropic_adapter_missing_key_error`

Acceptance Criteria:
- Canonical output only.
- No pricing or registry internal usage.
- `cargo check --lib` passes.

Depends On:
- Stage 16
- Stage 10
