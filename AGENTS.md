  # AGENTS.md

  ## Purpose
  `provider-runtime` is a Rust library that provides a canonical, provider-agnostic LLM runtime API for:
  - conversation messages
  - tool definitions and tool calls
  - structured output (JSON mode/schema)
  - usage accounting and optional cost tracking

  This library is designed to be a llm provider agnostic library for agent harnesses/workflows. It does **not** run tool loops or maintain session state itself.

  ## Source of Truth
  Before making changes, read:
  1. `docs/V0_Plan.md` (master plan)
  2. `docs/V0_StepByStepPlan.md` (current stage specific plans and ordering)
  3. `docs/Stage*.md` (stage-specific implementation contracts)

  If there is a conflict, treat `docs/V0_Plan.md` as canonical unless explicitly updated.

  ## Working Model
  - Implement by stage, in order.
  - Keep changes isolated to the active stage file(s) whenever possible.
  - Treat completed stage interfaces as frozen.
  - Do not redesign public API during implementation stages.

  ## Rust Commands
  Use these before submitting changes:

  ```bash
  cargo check --all-targets
  cargo fmt --all --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-targets --all-features

  (If needed to apply formatting:)

  cargo fmt --all

  ## PR / Change Expectations

  - Reference the stage being implemented (e.g., Stage 12).
  - Keep behavior deterministic.
  - Add/adjust tests required by that stage.
  - Preserve architectural invariants and non-goals above.
