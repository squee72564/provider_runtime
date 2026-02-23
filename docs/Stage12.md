# Stage 12 - ProviderRuntime and Builder Implementation

Goal:
Implement stateless runtime orchestration API exactly as specified.

Files:
- [src/runtime.rs](src/runtime.rs)

Public API Surface:
- `struct ProviderRuntime`
- `struct ProviderRuntimeBuilder`
- `impl ProviderRuntime::builder()`
- `async fn ProviderRuntime::run(...)`
- `async fn ProviderRuntime::discover_models(...)`
- `fn ProviderRuntime::export_catalog_json(...)`

Internal Responsibilities:
- Route request via registry.
- Invoke adapter and return canonical response.
- Attach optional cost estimate via pricing module.
- No loop orchestration, no session state.

Unit Tests:
- `test_runtime_run_routes_request`
- `test_runtime_cost_attached_when_pricing_available`
- `test_runtime_discover_models_static_first`

Acceptance Criteria:
- Public runtime methods match v0 API contract.
- Runtime remains stateless and async-only.
- `cargo check --lib` passes.

Depends On:
- Stage 7
- Stage 8
- Stage 11
