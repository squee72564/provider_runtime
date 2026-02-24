# Stage 19 - OpenRouter Adapter

Goal:
Implement OpenRouter `ProviderAdapter` with capability declaration and strict adapter-private configuration/transport orchestration boundaries.

Files:
- [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs)

Public API Surface:
- `struct OpenRouterAdapter`
- `struct OpenRouterAdapterOptions`
- `impl ProviderAdapter for OpenRouterAdapter`

Internal Responsibilities:
- Capability declaration.
- Run/discover via transport + auth + translator composition.
- Adapter option validation for expanded non-streaming OpenRouter surface.
- Enforce strict unsupported-mode policy at adapter option boundary for out-of-scope streaming/multimodal request knobs.
- Use provider-reported cost as response metadata input only.
- Adapter orchestration must not perform ad hoc canonical<->provider field mapping.
- All protocol mapping must go through the shared crate-private translator contract.

Unit Tests:
- `test_openrouter_adapter_capabilities`
- `test_openrouter_adapter_missing_key_error`
- `test_openrouter_adapter_uses_translator_boundary`
- option-validation tests for expanded adapter options
- request-body assertion tests verifying extended options are forwarded through translator payloads

Acceptance Criteria:
- Adapter does not call pricing engine directly.
- Canonical output contract preserved.
- Adapter responsibilities are limited to orchestration (auth/transport/capabilities/error envelope handling + option validation).
- OpenRouter-specific knobs stay adapter-private and do not leak into canonical request/response types.
- `cargo check --lib` passes.

Depends On:
- Stage 18
- Stage 10
