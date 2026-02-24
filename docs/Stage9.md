# Stage 9 - Deterministic Handoff Normalization

Goal:
Implement cross-provider handoff conversion rules.

Files:
- `src/handoff/mod.rs` (not in repo yet)

Public API Surface:
- `fn normalize_handoff_messages(...) -> Vec<Message>`

Internal Responsibilities:
- Preserve user messages/tool results unchanged.
- Preserve same-provider assistant blocks as-is.
- Convert cross-provider thinking blocks to tagged text.
- Preserve tool calls and normal text unchanged.

Unit Tests:
- `test_same_provider_assistant_preserved`
- `test_cross_provider_thinking_to_tagged_text`
- `test_tool_calls_and_results_preserved`

Acceptance Criteria:
- Transform is deterministic and pure.
- No provider protocol JSON leakage.
- `cargo check --lib` passes.

Depends On:
- Stage 1
