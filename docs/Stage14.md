# Stage 14 - OpenAI Translator

Goal:
Implement canonical-to-OpenAI and OpenAI-to-canonical translation.

Files:
- [src/providers/openai_translate/mod.rs](src/providers/openai_translate/mod.rs)

Public API Surface:
- `fn encode_openai_request(...)`
- `fn decode_openai_response(...)`

Internal Responsibilities:
- Implement OpenAI translator using shared crate-private translator contract semantics from Stage 3.
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
- Keep provider raw JSON internal to adapter layer.
- Defer provider-specific field mapping tables to later implementation docs/tests.

Unit Tests:
- `test_encode_openai_translator_category_contract`
- `test_decode_openai_translator_category_contract`
- `test_openai_translator_determinism_contract`

Acceptance Criteria:
- Translator does not depend on registry or pricing internals.
- Translator conforms to shared translator contract semantics documented in Stage 3.
- Provider-specific field-level mapping details remain deferred (not specified in this stage doc).
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
