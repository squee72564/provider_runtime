# Stage 16 - Anthropic Translator

Goal:
Implement canonical-to-Anthropic and Anthropic-to-canonical translation.

Files:
- [src/providers/anthropic_translate.rs](src/providers/anthropic_translate.rs)

Public API Surface:
- `fn encode_anthropic_request(...)`
- `fn decode_anthropic_response(...)`

Internal Responsibilities:
- Map canonical tool schemas and messages to Anthropic format.
- Parse Anthropic usage and response blocks into canonical output.

Unit Tests:
- `test_encode_anthropic_tool_schema`
- `test_decode_anthropic_usage_and_blocks`

Acceptance Criteria:
- No registry/pricing dependency.
- `cargo check --lib` passes.

Depends On:
- Stage 4
- Stage 6
