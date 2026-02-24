# Deterministic Implementation Plan: `provider-runtime` v0

## 1. Architectural Assumptions (Explicit)
- No streaming in v0.
- Runtime is stateless.
- Runtime speaks one universal language internally
  - Each provider adaptor is a translator
  - We translate our universal request into providers native format, send over HTTP, then translate providers response back into universal format
  - Example:
    ```mmd
    flowchart TD
        subgraph Runtime
            A[Canonical Request]
        end

        subgraph Provider Adapter
            B[Translate to Provider Format]
            D[Normalize to Canonical]
        end

        subgraph External API
            C[HTTP Call]
        end

        A --> B --> C --> D --> E[Canonical Response]
    ```
- No tool loop orchestration in library code.
- Async-only execution surface.
- Canonical types are provider-agnostic.
- Pricing is optional; missing pricing yields warnings, not hard failure.
- Discovery is static-first with optional remote enrichment.
- Handoff normalization is deterministic.
- Provider adapters must not leak raw protocol shapes outside canonical response fields.
- OAuth flows are out of scope for v0 (token-provider hook allowed, flow implementation forbidden).
- Session persistence/state management is out of scope.

## 2. Global Dependency Graph
Topological stage ordering:

1. Core schema and contracts: Stages 1-4  
2. Transport foundation: Stages 5-6  
3. Cross-cutting deterministic logic: Stages 7-9  
4. Registry and routing: Stages 10-11  
5. Runtime orchestration API: Stages 12-13  
6. Provider adapter groups: Stages 14-20  
7. Contract tests: Stages 21-23  
8. Integration and acceptance: Stages 24-25

Dependency rules:
- Adapters require frozen canonical types and frozen `ProviderAdapter` trait (Stages 1-4 complete).
- Registry requires canonical model/request types and provider trait (Stages 1-4 complete).
- Runtime orchestration requires registry + pricing + catalog + handoff (Stages 7-11 complete).
- Pricing requires `Usage` canonical type (Stage 1 complete).
- Catalog logic requires canonical `ModelInfo/ModelCatalog/DiscoveryOptions` (Stage 1 complete) and is consumed by registry/runtime.
- Integration tests require runtime + registry + at least one real adapter (Stages 1-20 complete).
- No circular dependencies: providers depend on core/transport only; runtime depends on registry/pricing/catalog/handoff; registry depends on core traits only.

## 3. Stage Plan (Checkbox Format)

## [x] Stage 1 - Freeze Canonical Domain Types
Goal:
Define provider-agnostic canonical request/response/message/tool/usage/cost/catalog types in one place.

Files:
- [src/core/types/mod.rs](src/core/types/mod.rs)

Public API Surface:
- `struct ProviderRequest`
- `struct ProviderResponse`
- `struct ModelRef`
- `struct Message`
- `enum MessageRole`
- `enum ContentPart`
- `struct ToolDefinition`
- `struct ToolCall`
- `struct ToolResult`
- `enum ToolChoice`
- `enum ResponseFormat`
- `struct AssistantOutput`
- `struct Usage`
- `struct CostBreakdown`
- `enum PricingSource`
- `enum FinishReason`
- `struct RuntimeWarning`
- `struct ModelInfo`
- `struct ModelCatalog`
- `struct DiscoveryOptions`
- `enum ProviderId`
- `struct ProviderCapabilities`
- `struct AdapterContext`

Stage Documentation: `docs/Stage1.md`

Depends On:
- None

## [x] Stage 2 - Runtime and Provider Error Taxonomy
Goal:
Create deterministic error model for config, routing, transport, provider protocol, and cost warnings.

Files:
- [src/core/error/mod.rs](src/core/error/mod.rs)

Public API Surface:
- `enum RuntimeError`
- `enum ProviderError`
- `enum ConfigError`
- `enum RoutingError`

Stage Documentation: `docs/Stage2.md`

Depends On:
- Stage 1

## [ ] Stage 3 - Freeze Core Traits
Goal:
Define stable adapter/auth contracts used by all providers and runtime.

Files:
- [src/core/traits/mod.rs](src/core/traits/mod.rs)

Public API Surface:
- `trait ProviderAdapter`
- `trait TokenProvider`

Stage Documentation: `docs/Stage3.md`

Depends On:
- Stage 1
- Stage 2

## [x] Stage 4 - Core Module Exports and Freeze Gate
Goal:
Export frozen core interfaces and lock core boundary.

Files:
- [src/core/mod.rs](src/core/mod.rs)

Public API Surface:
- `pub mod types`
- `pub mod traits`
- `pub mod error`

Stage Documentation: `docs/Stage4.md`

Depends On:
- Stage 1
- Stage 2
- Stage 3

## [ ] Stage 5 - HTTP Transport Abstraction
Goal:
Implement shared async HTTP utility used by adapters.

Files:
- [src/transport/http/mod.rs](src/transport/http/mod.rs)

Public API Surface:
- `struct HttpTransport`
- `struct RetryPolicy`
- `impl HttpTransport` methods for JSON request/response

Stage Documentation: `docs/Stage5.md`

Depends On:
- Stage 2
- Stage 4

## [ ] Stage 6 - Transport Module Export
Goal:
Expose transport module boundary.

Files:
- [src/transport/mod.rs](src/transport/mod.rs)

Public API Surface:
- `pub mod http`

Stage Documentation: `docs/Stage6.md`

Depends On:
- Stage 5

## [ ] Stage 7 - Pricing Engine (Optional, Warning-Based)
Goal:
Implement cost estimation from canonical usage + configurable pricing table.

Files:
- `src/pricing/mod.rs` (not in repo yet)

Public API Surface:
- `struct PricingTable`
- `struct PriceRule`
- `fn estimate_cost(...) -> (Option<CostBreakdown>, Vec<RuntimeWarning>)`

Stage Documentation: `docs/Stage7.md`

Depends On:
- Stage 1
- Stage 2

## [ ] Stage 8 - Static-First Catalog Logic
Goal:
Implement model catalog merge/lookup/export behavior with static-first policy.

Files:
- `src/catalog/mod.rs` (not in repo yet)

Public API Surface:
- `fn merge_static_and_remote_catalog(...) -> ModelCatalog`
- `fn resolve_model_provider(...) -> Result<ProviderId, RoutingError>`
- `fn export_catalog_json(...) -> Result<String, RuntimeError>`

Stage Documentation: `docs/Stage8.md`

Depends On:
- Stage 1
- Stage 2

## [ ] Stage 9 - Deterministic Handoff Normalization
Goal:
Implement cross-provider handoff conversion rules.

Files:
- `src/handoff/mod.rs` (not in repo yet)

Public API Surface:
- `fn normalize_handoff_messages(...) -> Vec<Message>`

Stage Documentation: `docs/Stage9.md`

Depends On:
- Stage 1

## [ ] Stage 10 - Provider Registry Core
Goal:
Implement provider registration and model-to-provider routing.

Files:
- [src/registry/registry/mod.rs](src/registry/registry/mod.rs)

Public API Surface:
- `struct ProviderRegistry`
- `fn register(...)`
- `fn resolve_adapter(...)`
- `fn resolve_provider(...)`
- `async fn discover_models(...)`

Stage Documentation: `docs/Stage10.md`

Depends On:
- Stage 3
- Stage 8

## [ ] Stage 11 - Registry Module Export
Goal:
Expose registry boundary.

Files:
- [src/registry/mod.rs](src/registry/mod.rs)

Public API Surface:
- `pub mod registry`

Stage Documentation: `docs/Stage11.md`

Depends On:
- Stage 10

## [ ] Stage 12 - ProviderRuntime and Builder Implementation
Goal:
Implement stateless runtime orchestration API exactly as specified.

Files:
- `src/runtime/mod.rs` (not in repo yet)

Public API Surface:
- `struct ProviderRuntime`
- `struct ProviderRuntimeBuilder`
- `impl ProviderRuntime::builder()`
- `async fn ProviderRuntime::run(...)`
- `async fn ProviderRuntime::discover_models(...)`
- `fn ProviderRuntime::export_catalog_json(...)`

Stage Documentation: `docs/Stage12.md`

Depends On:
- Stage 7
- Stage 8
- Stage 11

## [ ] Stage 13 - Crate Exports and Runtime Freeze Gate
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

Stage Documentation: `docs/Stage13.md`

Depends On:
- Stage 12

## [ ] Stage 14 - OpenAI Translator
Goal:
Implement canonical-to-OpenAI and OpenAI-to-canonical translation.

Files:
- [src/providers/openai/mod.rs](src/providers/openai/mod.rs)

Public API Surface:
- `fn encode_openai_request(...)`
- `fn decode_openai_response(...)`

Stage Documentation: `docs/Stage14.md`

Depends On:
- Stage 4
- Stage 6

## [ ] Stage 15 - OpenAI Adapter
Goal:
Implement OpenAI `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openai/mod.rs](src/providers/openai/mod.rs)

Public API Surface:
- `struct OpenAiAdapter`
- `impl ProviderAdapter for OpenAiAdapter`

Stage Documentation: `docs/Stage15.md`

Depends On:
- Stage 14
- Stage 10

## [ ] Stage 16 - Anthropic Translator
Goal:
Implement canonical-to-Anthropic and Anthropic-to-canonical translation.

Files:
- [src/providers/anthropic/mod.rs](src/providers/anthropic/mod.rs)

Public API Surface:
- `fn encode_anthropic_request(...)`
- `fn decode_anthropic_response(...)`

Stage Documentation: `docs/Stage16.md`

Depends On:
- Stage 4
- Stage 6

## [ ] Stage 17 - Anthropic Adapter
Goal:
Implement Anthropic `ProviderAdapter` with capability declaration.

Files:
- [src/providers/anthropic/mod.rs](src/providers/anthropic/mod.rs)

Public API Surface:
- `struct AnthropicAdapter`
- `impl ProviderAdapter for AnthropicAdapter`

Stage Documentation: `docs/Stage17.md`

Depends On:
- Stage 16
- Stage 10

## [ ] Stage 18 - OpenRouter Translator
Goal:
Implement canonical-to-OpenRouter and OpenRouter-to-canonical translation.

Files:
- [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs)

Public API Surface:
- `fn encode_openrouter_request(...)`
- `fn decode_openrouter_response(...)`

Stage Documentation: `docs/Stage18.md`

Depends On:
- Stage 4
- Stage 6

## [ ] Stage 19 - OpenRouter Adapter
Goal:
Implement OpenRouter `ProviderAdapter` with capability declaration.

Files:
- [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs)

Public API Surface:
- `struct OpenRouterAdapter`
- `impl ProviderAdapter for OpenRouterAdapter`

Stage Documentation: `docs/Stage19.md`

Depends On:
- Stage 18
- Stage 10

## [ ] Stage 20 - Providers Module Wiring
Goal:
Expose provider adapters/translators without leaking internals.

Files:
- [src/providers/mod.rs](src/providers/mod.rs)

Public API Surface:
- `pub mod openai`
- `pub mod anthropic`
- `pub mod openrouter`
- `pub(crate) mod openai_translate`
- `pub(crate) mod anthropic_translate`
- `pub(crate) mod openrouter_translate`

Stage Documentation: `docs/Stage20.md`

Depends On:
- Stage 15
- Stage 17
- Stage 19

## [ ] Stage 21 - OpenAI Contract Fixtures
Goal:
Add canonical↔OpenAI golden fixture contract tests.

Files:
- `tests/contract_openai.rs` (not in repo yet)

Public API Surface:
- None

Stage Documentation: `docs/Stage21.md`

Depends On:
- Stage 20

## [ ] Stage 22 - Anthropic Contract Fixtures
Goal:
Add canonical↔Anthropic golden fixture contract tests.

Files:
- `tests/contract_anthropic.rs` (not in repo yet)

Public API Surface:
- None

Stage Documentation: `docs/Stage22.md`

Depends On:
- Stage 20

## [ ] Stage 23 - OpenRouter Contract Fixtures
Goal:
Add canonical↔OpenRouter golden fixture contract tests.

Files:
- `tests/contract_openrouter.rs` (not in repo yet)

Public API Surface:
- None

Stage Documentation: `docs/Stage23.md`

Depends On:
- Stage 20

## [ ] Stage 24 - Runtime Integration Tests (Mock HTTP)
Goal:
Validate registry+runtime+adapters end-to-end with mocked provider endpoints.

Files:
- `tests/integration_runtime_mock_http.rs` (not in repo yet)

Public API Surface:
- None

Stage Documentation: `docs/Stage24.md`

Depends On:
- Stage 13
- Stage 20
- Stage 23

## [ ] Stage 25 - Acceptance: Multi-Provider Canonical Behavior
Goal:
Final acceptance suite for v0 invariants across providers and handoffs.

Files:
- `tests/acceptance_multi_provider_run.rs` (not in repo yet)

Public API Surface:
- None

Stage Documentation: `docs/Stage25.md`

Depends On:
- Stage 9
- Stage 24

## 4. Module Isolation Requirements

Core (`core/*`):
- Black box: canonical schema + contracts + errors only.
- Forbidden leak: no HTTP, no provider JSON, no pricing algorithms.
- Freeze point: end of Stage 4.

Transport (`transport/*`):
- Black box: request execution/retry/timeout.
- Forbidden leak: no canonical translation or routing decisions.
- Freeze point: end of Stage 6 for adapter consumption.

Pricing (`pricing/*`):
- Black box: deterministic cost math from usage + table.
- Forbidden leak: no provider request/response parsing, no routing.
- Freeze point: end of Stage 7 output contract.

Catalog (`catalog/*`):
- Black box: model catalog merge/resolve/export policy.
- Forbidden leak: no HTTP calls and no adapter internals.
- Freeze point: end of Stage 8 behavior contract.

Handoff (`handoff/*`):
- Black box: pure message transformation.
- Forbidden leak: no transport/adapters/routing coupling.
- Freeze point: end of Stage 9 deterministic transformation contract.

Registry (`registry/*`):
- Black box: provider registration and model routing.
- Forbidden leak: no protocol translation and no pricing computation.
- Freeze point: end of Stage 11 route/lookup API.

Providers (`providers/*`):
- Black box: canonical <-> provider translation + API invocation.
- Forbidden leak: no registry internals, no pricing internals, no session logic.
- Freeze point: each adapter API at its stage completion.

Runtime (`runtime/*`):
- Black box: stateless orchestration around registry/adapters/pricing/catalog.
- Forbidden leak: no streaming, no loops, no persistent conversation state.
- Freeze point: end of Stage 13 public methods.

## 5. Adapter Implementation Strategy

OpenAI stage group:
- Translator file: [src/providers/openai/mod.rs](src/providers/openai/mod.rs) (Stage 14)
- Adapter struct + capabilities: [src/providers/openai/mod.rs](src/providers/openai/mod.rs) (Stage 15)
- Golden fixtures: `tests/contract_openai.rs` (not in repo yet) (Stage 21)

Anthropic stage group:
- Translator file: [src/providers/anthropic/mod.rs](src/providers/anthropic/mod.rs) (Stage 16)
- Adapter struct + capabilities: [src/providers/anthropic/mod.rs](src/providers/anthropic/mod.rs) (Stage 17)
- Golden fixtures: `tests/contract_anthropic.rs` (not in repo yet) (Stage 22)

OpenRouter stage group:
- Translator file: [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs) (Stage 18)
- Adapter struct + capabilities: [src/providers/openrouter/mod.rs](src/providers/openrouter/mod.rs) (Stage 19)
- Golden fixtures: `tests/contract_openrouter.rs` (not in repo yet) (Stage 23)

Hard rules for all adapters:
- Depend only on canonical types/contracts and `AdapterContext` + transport.
- No direct coupling to registry internals.
- No direct coupling to pricing internals.
- No raw protocol structs in public API.

## 6. Testing Phases

Phase 1: Unit tests (per stage)
- Embedded or module-local tests added with each implementation stage (1-20).
- Purpose: isolate behavior by module with minimal dependencies.

Phase 2: Contract tests (canonical <-> provider fixtures)
- Stages 21-23 only after provider modules are complete.
- Fixture-based encode/decode determinism for each provider.

Phase 3: Integration tests (mock HTTP)
- Stage 24 only after runtime + registry + adapters are complete.
- Validate orchestration and routing without external provider calls.

Phase 4: Acceptance tests (multi-provider run)
- Stage 25 final gate.
- Validate v0 invariants across end-to-end canonical behavior.

## 7. Freeze Points

Freeze Point A (end of Stage 4):
- Canonical types in `core/types/mod.rs` are frozen.
- `RuntimeError`/`ProviderError` taxonomy is frozen.
- Later stages may not change field names or semantic meaning.

Freeze Point B (end of Stage 4):
- `ProviderAdapter` trait signature is frozen.
- Later stages may implement trait only; no signature edits.

Freeze Point C (end of Stage 13):
- `ProviderRuntime` public API (`builder`, `run`, `discover_models`, `export_catalog_json`) is frozen.
- Later stages may not change runtime method signatures or behavior contract.

## 8. Final Validation Checklist
- [ ] No streaming APIs, types, or behavior introduced.
- [ ] No session state or persistence introduced.
- [ ] No tool-loop orchestration introduced.
- [ ] Canonical layer remains provider-agnostic and protocol-shape independent.
- [ ] Handoff normalization is deterministic and tested.
- [ ] Pricing remains optional; missing pricing produces warnings only.
- [ ] Discovery remains static-first with optional remote enrichment.
- [ ] OpenAI adapter passes unit + contract + integration tests.
- [ ] Anthropic adapter passes unit + contract + integration tests.
- [ ] OpenRouter adapter passes unit + contract + integration tests.
- [ ] Runtime `run()` remains stateless and async-only.
- [ ] All integration and acceptance tests pass after Stage 25.
