# Stage 19 - OpenRouter Adapter

Goal:
Implement OpenRouter `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs)

Public API Surface:
- `struct OpenRouterAdapter`
- `impl ProviderAdapter for OpenRouterAdapter`

Internal Responsibilities:
- Capability declaration.
- Run/discover via transport + auth + translator composition.
- Use provider-reported cost as response metadata input only.
- Adapter orchestration must not perform ad hoc canonical<->provider field mapping.
- All protocol mapping must go through the shared crate-private translator contract.

Unit Tests:
- `test_openrouter_adapter_capabilities`
- `test_openrouter_adapter_missing_key_error`
- `test_openrouter_adapter_uses_translator_boundary`

Acceptance Criteria:
- Adapter does not call pricing engine directly.
- Canonical output contract preserved.
- Adapter responsibilities are limited to orchestration (auth/transport/capabilities/error envelope handling).
- `cargo check --lib` passes.

Depends On:
- Stage 18
- Stage 10
