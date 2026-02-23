# Stage 6 - Transport Module Export

Goal:
Expose transport module boundary.

Files:
- [src/transport/mod.rs](src/transport/mod.rs)

Public API Surface:
- `pub mod http`

Internal Responsibilities:
- Export wiring only.

Unit Tests:
- `test_transport_exports_compile`

Acceptance Criteria:
- Transport is available to adapters/runtime.
- `cargo check --lib` passes.

Depends On:
- Stage 5
