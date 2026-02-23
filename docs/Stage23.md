# Stage 23 - OpenRouter Contract Fixtures

Goal:
Add canonical↔OpenRouter golden fixture contract tests.

Files:
- [tests/contract_openrouter.rs](tests/contract_openrouter.rs)

Public API Surface:
- None

Internal Responsibilities:
- Fixture-driven encode/decode validation.

Unit Tests:
- `test_openrouter_encode_fixture_contract`
- `test_openrouter_decode_fixture_contract`

Acceptance Criteria:
- OpenRouter translator contract is deterministic.
- `cargo test --test contract_openrouter` passes.

Depends On:
- Stage 20
