# Stage 14 - OpenAI Translator

Goal:
Implement deterministic canonical-to-OpenAI and OpenAI-to-canonical translation for the OpenAI Responses API surface.

Files:
- [src/providers/openai_translate/mod.rs](src/providers/openai_translate/mod.rs)

Public API Surface:
- `fn encode_openai_request(...)`
- `fn decode_openai_response(...)`

Internal Types (crate-private):
- `OpenAiEncodedRequest { body: serde_json::Value, warnings: Vec<RuntimeWarning> }`
- `OpenAiDecodeEnvelope { body: serde_json::Value, requested_response_format: ResponseFormat }`

Internal Responsibilities:
- Implement OpenAI translation using shared crate-private translator contract semantics from Stage 3.
- Keep translator isolated from runtime orchestration, pricing, and registry internals.
- Encode coverage:
  - minimal text and multi-message inputs
  - tools + tool choice (`none`, `auto`, `required`, `specific{name}`)
  - response formats (`text`, `json_object`, `json_schema`)
  - optional controls (`temperature`, `top_p`, `max_output_tokens`, `metadata`)
  - force `store: false` on OpenAI Responses API requests to avoid provider-side persisted context in v0
  - deterministic unsupported handling for canonical `stop`
- Decode coverage:
  - assistant text output
  - tool-call output
  - refusal mapping to canonical text
  - ignore provider reasoning blocks (not canonicalized)
  - structured output extraction using request context envelope
  - usage mapping for full/partial/absent usage payloads
  - finish-reason mapping from `status` + `incomplete_details.reason`

Deterministic Validation / Error Policy:
- `provider_hint` mismatch, empty model id, unsupported stop, invalid metadata bounds, and invalid role/content combinations return deterministic `ProviderError::Protocol`.
- Tool schema strict compatibility is checked deterministically:
  - strict-compatible schemas encode with `strict: true`
  - non-compatible schemas encode with `strict: false` and stable warning
- `ToolChoice::Specific { name }` requires a matching tool definition name or deterministic protocol error.
- Non-completed statuses (`cancelled`, `queued`, `in_progress`, `failed`) decode to deterministic protocol errors.
- Unsupported payload shapes and unknown output item types return deterministic protocol errors.

Warning Policy:
- Stable warning code + message for same condition.
- Required warning categories:
  - both temperature and top_p set
  - non-strict tool schema strict disabled
  - tool arguments invalid JSON fallback
  - usage missing
  - refusal mapped to text
  - structured output parse failure
  - incomplete reasons (`max_output_tokens`, `content_filter`, unknown/missing)
  - empty output

Determinism Requirements:
- Equal canonical input encodes identically.
- Equal provider payload decodes identically.
- Warning/error behavior is stable for equal input.
- Provider raw JSON remains internal (translator returns canonical response with `raw_provider_response = None`).

Unit Tests:
- `test_encode_openai_translator_category_contract`
- `test_decode_openai_translator_category_contract`
- `test_openai_translator_determinism_contract`
- plus focused edge tests for unsupported intent and decode edge states

Acceptance Criteria:
- Translator conforms to crate-private contract semantics.
- Translator surface fully covers Stage 14 category matrix.
- Deterministic warning/error behavior is verified by tests.
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
