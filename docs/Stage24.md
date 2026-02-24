# Stage 24 - Runtime Integration Tests (Mock HTTP)

Goal:
Validate registry+runtime+adapters end-to-end with mocked provider endpoints.

Files:
- `tests/integration_runtime_mock_http.rs` (not in repo yet)

Public API Surface:
- None

Internal Responsibilities:
- Mock HTTP runs for each provider.
- Validate routing, canonical response shape, optional cost warnings.

Unit Tests:
- `test_runtime_run_openai_mock`
- `test_runtime_run_anthropic_mock`
- `test_runtime_run_openrouter_mock`
- `test_runtime_routing_and_warning_behavior`

Acceptance Criteria:
- Runtime orchestrates without session state or loop logic.
- `cargo test --test integration_runtime_mock_http` passes.

Depends On:
- Stage 13
- Stage 20
- Stage 23
