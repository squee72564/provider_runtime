# Stage 23 - OpenRouter Contract Fixtures

Goal:
Add canonical↔OpenRouter golden fixture contract tests.

Files:
- `tests/contract_openrouter.rs` (not in repo yet)

Public API Surface:
- None

Internal Responsibilities:
- Fixture-driven encode/decode validation.
- Require provider-agnostic fixture category matrix:
  - canonical request categories:
    - minimal text request
    - multi-message conversation
    - tools with each tool-choice mode
    - response formats (`text`, `json_object`, `json_schema`)
    - optional controls present/absent (`temperature`, `top_p`, `max_output_tokens`, `stop`, `metadata`)
  - provider response categories:
    - text-only assistant output
    - tool-call output
    - structured output present
    - usage fields partial/full/absent
    - finish reason normalization
  - error/edge categories:
    - protocol error payload -> canonical error mapping
    - unsupported canonical intent -> deterministic error or warning
    - malformed payload decode failure
  - determinism categories:
    - stable encode output for identical canonical input
    - stable decode output for identical provider payload
    - stable warning/error code behavior for same failure mode

Unit Tests:
- `test_openrouter_encode_fixture_contract`
- `test_openrouter_decode_fixture_contract`
- `test_openrouter_fixture_category_matrix_coverage`

Acceptance Criteria:
- OpenRouter translator contract is deterministic.
- Fixtures satisfy shared provider-agnostic category matrix requirements.
- `cargo test --test contract_openrouter` passes.

Depends On:
- Stage 20
