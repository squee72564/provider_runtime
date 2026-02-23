# Stage 15 - OpenAI Adapter

Goal:
Implement OpenAI `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openai.rs](src/providers/openai.rs)

Public API Surface:
- `struct OpenAiAdapter`
- `impl ProviderAdapter for OpenAiAdapter`

Internal Responsibilities:
- Declare capabilities.
- Use transport + translator for run/discover_models.
- Credential validation and provider-specific error mapping.

Unit Tests:
- `test_openai_adapter_capabilities`
- `test_openai_adapter_missing_key_error`

Acceptance Criteria:
- Adapter returns canonical response only.
- Adapter has no pricing or registry internal coupling.
- `cargo check --lib` passes.

Depends On:
- Stage 14
- Stage 10
