# Stage 18 - OpenRouter Translator

Goal:
Implement canonical-to-OpenRouter and OpenRouter-to-canonical translation.

Files:
- [src/providers/openrouter_translate.rs](src/providers/openrouter_translate.rs)

Public API Surface:
- `fn encode_openrouter_request(...)`
- `fn decode_openrouter_response(...)`

Internal Responsibilities:
- OpenAI-compatible request mapping with compat flags.
- Parse token usage and provider-reported cost fields when present.

Unit Tests:
- `test_encode_openrouter_compat_settings`
- `test_decode_openrouter_usage_and_cost_fields`

Acceptance Criteria:
- Translator remains provider-protocol isolated.
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
