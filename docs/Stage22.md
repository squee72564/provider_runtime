# Stage 22 - Anthropic Contract Fixtures

Goal:
Add canonical↔Anthropic golden fixture contract tests.

Files:
- [tests/contract_anthropic.rs](tests/contract_anthropic.rs)

Public API Surface:
- None

Internal Responsibilities:
- Fixture-driven encode/decode validation.

Unit Tests:
- `test_anthropic_encode_fixture_contract`
- `test_anthropic_decode_fixture_contract`

Acceptance Criteria:
- Anthropic translator contract is deterministic.
- `cargo test --test contract_anthropic` passes.

Depends On:
- Stage 20
