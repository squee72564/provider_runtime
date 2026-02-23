# Stage 13 - Crate Exports and Runtime Freeze Gate

Goal:
Wire crate-level modules and freeze public API surface.

Files:
- [src/lib.rs](src/lib.rs)

Public API Surface:
- `pub mod core`
- `pub mod transport`
- `pub mod registry`
- `pub mod providers`
- `pub mod pricing`
- `pub mod catalog`
- `pub mod handoff`
- `pub mod runtime`
- `pub use runtime::{ProviderRuntime, ProviderRuntimeBuilder}`

Internal Responsibilities:
- Export wiring only.

Unit Tests:
- `test_public_api_compiles`

Acceptance Criteria:
- Runtime API freeze point established.
- No further signature changes after this stage.
- `cargo check --lib` passes.

Depends On:
- Stage 12
