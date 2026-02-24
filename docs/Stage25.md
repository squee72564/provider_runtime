# Stage 25 - Acceptance: Multi-Provider Canonical Behavior

Goal:
Final acceptance suite for v0 invariants across providers and handoffs.

Files:
- `tests/acceptance_multi_provider_run.rs` (not in repo yet)

Public API Surface:
- None

Internal Responsibilities:
- Multi-provider run scenarios.
- Cross-provider handoff determinism checks.
- Optional pricing behavior checks.

Unit Tests:
- `test_multi_provider_canonical_equivalence`
- `test_handoff_deterministic_behavior`
- `test_optional_cost_never_blocks_success`

Acceptance Criteria:
- All v0 constraints verified in one suite.
- `cargo test --test acceptance_multi_provider_run` passes.

Depends On:
- Stage 9
- Stage 24
