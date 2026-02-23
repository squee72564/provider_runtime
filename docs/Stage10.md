# Stage 10 - Provider Registry Core

Goal:
Implement provider registration and model-to-provider routing.

Files:
- [src/registry/registry.rs](src/registry/registry.rs)

Public API Surface:
- `struct ProviderRegistry`
- `fn register(...)`
- `fn resolve_adapter(...)`
- `fn resolve_provider(...)`
- `async fn discover_models(...)`

Internal Responsibilities:
- Adapter map lifecycle.
- Routing precedence: explicit provider hint, catalog mapping, default provider.
- Ambiguity/missing model deterministic errors.

Unit Tests:
- `test_registry_register_and_lookup`
- `test_routing_precedence_order`
- `test_ambiguous_model_returns_error`

Acceptance Criteria:
- Registry isolated from pricing logic.
- Registry depends only on core traits/types and catalog helpers.
- `cargo check --lib` passes.

Depends On:
- Stage 3
- Stage 8
