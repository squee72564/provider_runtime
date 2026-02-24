# Stage 18 - OpenRouter Translator

Goal:
Implement canonical-to-OpenRouter and OpenRouter-to-canonical translation.

Files:
- [src/providers/openrouter_translate/mod.rs](src/providers/openrouter_translate/mod.rs)

Public API Surface:
- `fn encode_openrouter_request(...)`
- `fn decode_openrouter_response(...)`

Internal Responsibilities:
- Implement OpenRouter translator using shared crate-private translator contract semantics from Stage 3.
- Canonical input coverage categories:
  - minimal text request
  - multi-message conversation
  - tools with each tool-choice mode
  - response formats (`text`, `json_object`, `json_schema`)
  - optional controls present/absent (`temperature`, `top_p`, `max_output_tokens`, `stop`, `metadata`)
- Decode normalization categories:
  - text-only assistant output
  - tool-call output
  - structured output present
  - usage fields partial/full/absent
  - finish reason normalization
- Unsupported/partial feature policy:
  - deterministic error or warning path for unsupported canonical intent
  - no silent lossy conversion
- Error mapping policy:
  - provider protocol error payloads normalize to `ProviderError`
  - malformed provider payloads return deterministic decode/protocol errors
- Warning emission policy:
  - stable warning code/message behavior for the same lossy/unsupported condition
- Determinism requirements:
  - equal canonical input encodes identically
  - equal provider payload decodes identically
- Defer provider-specific field mapping tables to later implementation docs/tests.

Unit Tests:
- `test_encode_openrouter_translator_category_contract`
- `test_decode_openrouter_translator_category_contract`
- `test_openrouter_translator_determinism_contract`

Acceptance Criteria:
- Translator remains provider-protocol isolated.
- Translator conforms to shared translator contract semantics documented in Stage 3.
- Provider-specific field-level mapping details remain deferred (not specified in this stage doc).
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
