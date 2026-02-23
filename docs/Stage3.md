# Stage 3 - Freeze Core Traits

Goal:
Define stable adapter/auth contracts used by all providers and runtime.

Files:
- [src/core/traits.rs](src/core/traits.rs)

Public API Surface:
- `trait ProviderAdapter`
- `trait TokenProvider`

Internal Responsibilities:
- `ProviderAdapter::{id, capabilities, run, discover_models}` signatures.
- Async trait object safety.
- Token-provider hook only, no OAuth flow logic.

Unit Tests:
- `test_provider_adapter_object_safety`
- `test_provider_capabilities_contract`

Acceptance Criteria:
- Trait signatures support all v0 providers without redesign.
- No pricing or registry internals in trait contract.
- `cargo check --lib` passes.

Depends On:
- Stage 1
- Stage 2
