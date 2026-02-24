# Stage 3 - Freeze Core Traits

Goal:
Define stable adapter/auth contracts used by all providers and runtime, and document the shared crate-private translator boundary used by provider modules.

Files:
- [src/core/traits/mod.rs](src/core/traits/mod.rs)

Public API Surface:
- `trait ProviderAdapter`
- `trait TokenProvider`

Internal Contract Surface (crate-private; not public API):
- `trait ProviderTranslator` in provider-layer internal module

Internal Responsibilities:
- `ProviderAdapter::{id, capabilities, run, discover_models}` signatures.
- Async trait object safety.
- Token-provider hook only, no OAuth flow logic.
- Define translator contract boundary for provider modules:
  - Associated provider request/response payload types.
  - `encode(canonical request) -> provider payload`.
  - `decode(provider payload) -> canonical response`.
  - protocol error payload normalization to `ProviderError`.
  - deterministic warning semantics for lossy/unsupported canonical intent.
- Clarify isolation:
  - Translator contract is crate-private and must not be re-exported.
  - Translator depends on canonical types + provider protocol schema only.
  - Translator must not depend on runtime orchestration, registry internals, or pricing internals.

Unit Tests:
- `test_provider_adapter_object_safety`
- `test_provider_capabilities_contract`
- Stage-level contract conformance expectation is validated by translator fixture stages (Stages 21-23), not by new core trait implementation tests.

Acceptance Criteria:
- Trait signatures support all v0 providers without redesign.
- No pricing or registry internals in trait contract.
- Shared translator contract semantics are documented and referenced by Stages 14/16/18.
- No new public API commitments added beyond `ProviderAdapter` and `TokenProvider`.
- `cargo check --lib` passes.

Depends On:
- Stage 1
- Stage 2
