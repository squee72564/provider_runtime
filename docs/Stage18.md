# Stage 18 - OpenRouter Translator

Goal:
Implement canonical-to-OpenRouter and OpenRouter-to-canonical translation for the OpenRouter Chat Completions API surface used in v0, with deterministic behavior and strict unsupported-mode handling.

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
- Adapter-private OpenRouter option coverage in encode path (non-streaming only):
  - `models` fallback list
  - `provider` preferences
  - `plugins`
  - `parallel_tool_calls`
  - `frequency_penalty`, `presence_penalty`
  - `logit_bias`, `logprobs`, `top_logprobs`
  - `reasoning`
  - `seed`
  - `user`, `session_id`, `trace`
  - deprecated compatibility fields: `route`, `max_tokens`
- Decode normalization categories:
  - text-only assistant output
  - tool-call output
  - structured output present
  - refusal text preserved in canonical content
  - usage fields partial/full/absent
  - finish reason normalization
- Unsupported/partial feature policy:
  - deterministic error path for unsupported canonical intent and unsupported non-streaming modes
  - no silent lossy conversion
- Error mapping policy:
  - provider protocol error payloads normalize to `ProviderError`
  - malformed provider payloads return deterministic decode/protocol errors
- Warning emission policy:
  - stable warning code/message behavior for partial/lossy conditions that are explicitly allowed
- Determinism requirements:
  - equal canonical input encodes identically
  - equal provider payload decodes identically

Unit Tests:
- `test_encode_openrouter_translator_category_contract`
- `test_decode_openrouter_translator_category_contract`
- `test_openrouter_translator_determinism_contract`
- validation tests for option bounds and unsupported non-streaming modes
- decode strictness tests for content-item type handling and tool-call type handling

Acceptance Criteria:
- Translator remains provider-protocol isolated.
- Translator conforms to shared translator contract semantics documented in Stage 3.
- OpenRouter-specific request knobs are adapter-private and do not leak into canonical types.
- Unsupported non-streaming-incompatible knobs return deterministic errors.
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
