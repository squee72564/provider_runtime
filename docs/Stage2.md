# Stage 2 - Runtime and Provider Error Taxonomy

Goal:
Create deterministic error model for config, routing, transport, provider protocol, and cost warnings.

Files:
- [src/core/error.rs](src/core/error.rs)

Public API Surface:
- `enum RuntimeError`
- `enum ProviderError`
- `enum ConfigError`
- `enum RoutingError`

Internal Responsibilities:
- Actionable missing-credential/provider/model error variants.
- Error context fields for provider/model/request-id.

Unit Tests:
- `test_runtime_error_display_messages`
- `test_missing_credential_error_contains_env_hints`

Acceptance Criteria:
- Errors map to v0 scope and non-goals.
- No provider-protocol shape leakage in error API.
- `cargo check --lib` passes.

Depends On:
- Stage 1
