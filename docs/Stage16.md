# Stage 16 - Anthropic Translator

Goal:
Implement canonical-to-Anthropic and Anthropic-to-canonical translation.

Files:
- [src/providers/anthropic_translate/mod.rs](src/providers/anthropic_translate/mod.rs)

Public API Surface:
- `fn encode_anthropic_request(...)`
- `fn decode_anthropic_response(...)`

Internal Responsibilities:
- Implement Anthropic translator using shared crate-private translator contract semantics from Stage 3.
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
- `test_encode_anthropic_translator_category_contract`
- `test_decode_anthropic_translator_category_contract`
- `test_anthropic_translator_determinism_contract`

Acceptance Criteria:
- No registry/pricing dependency.
- Translator conforms to shared translator contract semantics documented in Stage 3.
- Provider-specific field-level mapping details remain deferred (not specified in this stage doc).
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
