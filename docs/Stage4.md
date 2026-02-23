# Stage 4 - Core Module Exports and Freeze Gate

Goal:
Export frozen core interfaces and lock core boundary.

Files:
- [src/core/mod.rs](src/core/mod.rs)

Public API Surface:
- `pub mod types`
- `pub mod traits`
- `pub mod error`

Internal Responsibilities:
- Single source of truth exports for core interfaces.
- No logic.

Unit Tests:
- `test_core_exports_compile` (doctest or compile-only)

Acceptance Criteria:
- Canonical types and traits freeze point established.
- Later stages do not alter signatures.
- `cargo check --lib` passes.

Depends On:
- Stage 1
- Stage 2
- Stage 3
