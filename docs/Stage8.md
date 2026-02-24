# Stage 8 - Static-First Catalog Logic

Goal:
Implement model catalog merge/lookup/export behavior with static-first policy.

Files:
- `src/catalog/mod.rs` (not in repo yet)

Public API Surface:
- `fn merge_static_and_remote_catalog(...) -> ModelCatalog`
- `fn resolve_model_provider(...) -> Result<ProviderId, RoutingError>`
- `fn export_catalog_json(...) -> Result<String, RuntimeError>`

Internal Responsibilities:
- Deterministic merge precedence.
- Provider/model grouping.
- Stable JSON export ordering.

Unit Tests:
- `test_static_first_merge_policy`
- `test_resolve_model_provider_deterministic`
- `test_export_catalog_json_stable_output`

Acceptance Criteria:
- Discovery remains static-first.
- Runtime can export catalog without filesystem writes.
- `cargo check --lib` passes.

Depends On:
- Stage 1
- Stage 2
