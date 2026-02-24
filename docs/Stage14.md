# Stage 14 - OpenAI Translator

Goal:
Implement canonical-to-OpenAI and OpenAI-to-canonical translation.

Files:
- [src/providers/openai/mod.rs](src/providers/openai/mod.rs)

Public API Surface:
- `fn encode_openai_request(...)`
- `fn decode_openai_response(...)`

Internal Responsibilities:
- Map canonical messages/tools/response_format to Responses API shape.
- Parse usage and finish reason into canonical structures.
- Keep raw provider JSON internal to adapter layer.

Unit Tests:
- `test_encode_openai_tools_and_json_mode`
- `test_decode_openai_usage_mapping`

Acceptance Criteria:
- Translator does not depend on registry or pricing internals.
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
