# Stage 15 - OpenAI Adapter

Goal:
Implement OpenAI `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openai/mod.rs](src/providers/openai/mod.rs)

Public API Surface:
- `struct OpenAiAdapter`
- `impl ProviderAdapter for OpenAiAdapter`

Internal Responsibilities:
- Declare capabilities.
- Use transport + auth + translator composition for run/discover_models.
- Credential validation and provider-specific error mapping.
- Adapter orchestration must not perform ad hoc canonical<->provider field mapping.
- All protocol mapping must go through the shared crate-private translator contract.
- Remote discovery uses OpenAI `GET /v1/models` and maps discovered IDs into canonical `ModelInfo`
  with conservative defaults (`display_name/context_window/max_output_tokens = None`) because the
  endpoint does not provide complete catalog metadata for those fields.

Unit Tests:
- `test_openai_adapter_capabilities`
- `test_openai_adapter_missing_key_error`
- `test_openai_adapter_uses_translator_boundary`

Acceptance Criteria:
- Adapter returns canonical response only.
- Adapter has no pricing or registry internal coupling.
- Adapter responsibilities are limited to orchestration (auth/transport/capabilities/error envelope handling).
- `cargo check --lib` passes.

Depends On:
- Stage 14
- Stage 10
