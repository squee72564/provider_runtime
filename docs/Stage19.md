# Stage 19 - OpenRouter Adapter

Goal:
Implement OpenRouter `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openrouter.rs](src/providers/openrouter.rs)

Public API Surface:
- `struct OpenRouterAdapter`
- `impl ProviderAdapter for OpenRouterAdapter`

Internal Responsibilities:
- Capability declaration.
- Run/discover via transport + translator.
- Use provider-reported cost as response metadata input only.

Unit Tests:
- `test_openrouter_adapter_capabilities`
- `test_openrouter_adapter_missing_key_error`

Acceptance Criteria:
- Adapter does not call pricing engine directly.
- Canonical output contract preserved.
- `cargo check --lib` passes.

Depends On:
- Stage 18
- Stage 10
