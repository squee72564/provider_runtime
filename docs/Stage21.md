# Stage 21 - OpenAI Contract Fixtures

Goal:
Add canonical↔OpenAI golden fixture contract tests.

Files:
- [tests/contract_openai.rs](tests/contract_openai.rs)

Public API Surface:
- None

Internal Responsibilities:
- Fixture-driven encode/decode validation.
- Stability checks for canonical mapping.

Unit Tests:
- `test_openai_encode_fixture_contract`
- `test_openai_decode_fixture_contract`

Acceptance Criteria:
- OpenAI translator contract is deterministic.
- `cargo test --test contract_openai` passes.

Depends On:
- Stage 20
