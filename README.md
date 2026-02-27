Provider Runtime
===============

Provider Runtime is a Rust crate that normalizes the single-turn LLM experience across providers. It exposes a canonical request/response schema, deterministic routing logic, and optional usage/cost accounting so that agent harnesses can stay provider-agnostic without embedding session or orchestration state.

Why this crate exists
---------------------
- **Unified contract.** Every adapter speaks the same `ProviderRequest` / `ProviderResponse` shapes, shares canonical messages, tool definitions, structured output hints, and usage/cost data.
- **Provider routing.** A catalog-driven registry resolves models to providers, enforces capability checks (tools, structured output), and exposes optional discovery to enrich the catalog.
- **Deterministic runtime.** `ProviderRuntime::builder()` assembles adapters, catalog, pricing table, and context abstractions so `ProviderRuntime::run` performs one consistent, warning-aware request/response cycle.
- **Optional pricing.** Cost estimation runs when usage tokens exist and a pricing rule matches; otherwise the runtime surfaces warnings instead of panicking.

Repo snapshot
-------------
- `src/core`: canonical domain types (`Message`, `Usage`, `ModelCatalog`, etc.), traits (`ProviderAdapter`, `TokenProvider`), and error taxonomy that every consumer must build against.
- `src/catalog`: helpers for merging static/remote catalogs and exporting normalized JSON catalog snapshots.
- `src/registry`: the provider registry that wires adapters, resolves models, caches the active catalog, and coordinates discovery refreshes.
- `src/runtime`: the `ProviderRuntime`/`ProviderRuntimeBuilder` orchestration entry point plus runtime-focused tests.
- `src/pricing`: pricing rules, the `PricingTable`, and the warning-aware `estimate_cost` helper.
- `src/handoff`: helper for normalizing assistant `Thinking` content when handing off across providers.
- `src/transport`: HTTP transport abstractions, retry policies, and configurable headers/token handling that adapters rely on for provider calls.
- `src/providers`: in-progress provider adapters (OpenAI, Anthropic, OpenRouter) that implement the `ProviderAdapter` contract.

Testing & contributions
------------------------
- Tests live next to each module (`src/runtime/tests.rs`, `src/transport/tests.rs`, etc.). Keep additions focused on the stage you’re touching and reuse the `ProviderRuntime` builder to assert runtime behavior.
- The crate exports `ProviderRuntime`, `ProviderRuntimeBuilder`, and the canonical types from `core::types`, so keep breaking changes to those interfaces pegged to a new major version.

Live API smoke tests
--------------------
- Live tests are opt-in and cost-bearing. They are compiled only with `--features live-tests` and marked `ignored`.
- Run all live smoke tests:
  - `cargo test --features live-tests --test live_runtime_smoke -- --ignored --nocapture`
- Run a provider-specific subset:
  - `cargo test --features live-tests --test live_runtime_smoke live_openai -- --ignored --nocapture`
  - `cargo test --features live-tests --test live_runtime_smoke live_anthropic -- --ignored --nocapture`
  - `cargo test --features live-tests --test live_runtime_smoke live_openrouter -- --ignored --nocapture`
