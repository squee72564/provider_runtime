# Stage 9 - Deterministic Handoff Normalization

Goal:
Implement cross-provider handoff conversion rules.

Files:
- `src/handoff/mod.rs` (not in repo yet)

Public API Surface:
- `fn normalize_handoff_messages(...) -> Vec<Message>`

Internal Responsibilities:
- Preserve canonical messages unchanged.
- Preserve tool calls/results and text unchanged.

Unit Tests:
- `test_handoff_normalization_is_identity_for_assistant_content`
- `test_handoff_normalization_preserves_non_assistant_messages`
- `test_handoff_normalization_is_idempotent`

Acceptance Criteria:
- Transform is deterministic and pure.
- No provider protocol JSON leakage.
- `cargo check --lib` passes.

Depends On:
- Stage 1
