  # Provider Runtime v0 Plan (Canonical, Non-Streaming, Extensible)

  ## Summary

  Build provider-runtime as a standalone Rust library that normalizes core LLM semantics across providers:

  - Conversation messages
  - Tool definitions and model-emitted tool calls
  - Structured output / JSON mode
  - Usage accounting and cost estimation

  v0 ships with production adapters for:

  1. OpenAI
  2. Anthropic
  3. OpenRouter

  v0 explicitly excludes:

  - Streaming unification
  - Agent loop orchestration / tool execution orchestration
  - OAuth flow implementation (but includes auth extension points)

  The primary API is a single-turn async call: ProviderRuntime::run(request) -> ProviderResponse.

  ## Scope and Milestones

  1. Core canonical types + traits + runtime orchestration
  2. Registry + provider/model routing
  3. Auth/config system (builder + env fallback)
  4. Model catalog (static + optional remote discovery in-memory)
  5. Usage + cost tracking with configurable pricing tables
  6. Provider adapters: OpenAI, Anthropic, OpenRouter
  7. Cross-provider handoff normalization utility
  8. Test suite + docs + examples

  ## Public API / Interface Design

  ### 1) Runtime API

  - ProviderRuntime::builder() -> ProviderRuntimeBuilder
  - ProviderRuntime::run(&self, request: ProviderRequest) -> Result<ProviderResponse, RuntimeError>
  - ProviderRuntime::discover_models(&self, opts: DiscoveryOptions) -> Result<ModelCatalog, RuntimeError>
  - ProviderRuntime::export_catalog_json(&self, catalog: &ModelCatalog) -> Result<String, RuntimeError> (optional helper)

  No session state in runtime. Caller/harness owns conversation state and loop control.
  Where provider APIs support server-side response persistence, adapters should disable it by default in v0.

  ### 2) Canonical request/response types

  - ProviderRequest
      - model: ModelRef (provider_hint: Option<ProviderId>, model_id: String)
      - messages: Vec<Message>
      - tools: Vec<ToolDefinition>
      - tool_choice: ToolChoice (none/auto/required/specific{name})
      - response_format: ResponseFormat (text/json-schema/json-object)
      - temperature/top_p/max_output_tokens/stop (optional, provider-mapped)
      - metadata: HashMap<String, String>
  - ProviderResponse
      - output: AssistantOutput (text segments + tool calls + optional structured payload)
      - usage: Usage
      - cost: Option<CostBreakdown>
      - provider: ProviderId
      - model: String
      - raw_provider_response: Option<serde_json::Value> (debug opt-in)
      - finish_reason: FinishReason
      - warnings: Vec<RuntimeWarning>

  ### 3) Canonical message model

  - Message
      - role: MessageRole (System, User, Assistant, Tool)
      - content: Vec<ContentPart>
  - ContentPart
      - Text(String)
      - Thinking { text: String, provider: Option<ProviderId> }
      - ToolCall(ToolCall)
      - ToolResult(ToolResult)

  Handoff rule:

  - If assistant thinking originates from same provider/API family, keep as typed Thinking.
  - If crossing provider families, convert to text tags: <thinking>...</thinking> in Text.
  - Preserve tool calls and tool results unchanged.

  ### 4) Tools + structured output

  - ToolDefinition with JSON Schema parameters
  - ToolCall { id, name, arguments_json }
  - ToolResult {
      tool_call_id,
      content: ToolResultContent (Text | Json | Parts),
      raw_provider_content?: serde_json::Value
    }
  - ResponseFormat
      - Text
      - JsonObject
      - JsonSchema { name, schema }

  ### 5) Usage and cost

  - Usage
      - input_tokens, output_tokens, reasoning_tokens (optional), cached_input_tokens (optional), total_tokens (derived if missing)
  - CostBreakdown
      - currency: "USD"
      - input_cost, output_cost, reasoning_cost (optional), total_cost
      - pricing_source: PricingSource (Configured, ProviderReported, Mixed)
  - PricingTable keyed by (provider, model_pattern/version) with per-token rates
  - Cost calculation always runs when usage + matching price exist; otherwise cost: None with warning.

  ### 6) Provider/adapter abstractions

  - trait ProviderAdapter
      - fn id(&self) -> ProviderId
      - async fn run(&self, req: &ProviderRequest, ctx: &AdapterContext) -> Result<ProviderResponse, ProviderError>
      - async fn discover_models(&self, opts: &DiscoveryOptions, ctx: &AdapterContext) -> Result<Vec<ModelInfo>, ProviderError>
      - fn capabilities(&self) -> ProviderCapabilities
  - AdapterContext includes HTTP client, auth resolver, timeout/retry policy, pricing/catalog handles.

  ### 6a) Adapter vs Translator Boundaries

  Invariant:
  - Canonical models represent semantic intent and are provider-agnostic.
  - Provider wire formats are protocol-specific and are not exposed as canonical shapes.

  Boundary roles:
  - `ProviderAdapter` is orchestration-facing:
      - auth/headers
      - transport invocation
      - capability declaration
      - provider error envelope handling
  - Translator is protocol-facing and crate-private:
      - canonical request -> provider request payload
      - provider response payload -> canonical response
      - provider protocol error payload -> `ProviderError`
      - deterministic warning emission for lossy/unsupported conversions

  Translator contract (doc-level, crate-private):
  - Associated request/response protocol payload types per provider.
  - `encode(canonical_request) -> provider_request_payload`
  - `decode(provider_response_payload) -> canonical_response`
  - Returns typed `ProviderError` on protocol/serialization failures.

  Determinism requirements:
  - Input-equal canonical requests must encode identically.
  - Input-equal provider payloads must decode identically.
  - Unsupported canonical intent must map to deterministic error/warning behavior.

  Isolation requirements:
  - Translator depends on canonical types + provider protocol schema only.
  - Translator does not depend on registry internals, pricing internals, or runtime orchestration.

  ### 7) Registry/routing

  - ProviderRegistry
      - register adapters
      - resolve provider by:
          1. explicit provider_hint
          2. model mapping in catalog
          3. configured default provider
      - return deterministic errors for ambiguous or missing model routes.

  ### 8) Auth and config

  - ProviderRuntimeBuilder
      - .with_provider_config(...)
      - .with_api_key(...)
      - .with_token_provider(...) (future OAuth hook)
      - .with_timeout(...), .with_retry(...)
      - .with_pricing_table(...)
      - .with_model_catalog(...)
      - .with_env_fallback(true)
  - v0 auth behavior:
      - API key auth supported for shipped adapters.
      - Token-provider trait exists but OAuth flows are not implemented.
      - Missing credential errors are provider-specific and actionable.

  ### 9) Discovery model

  - DiscoveryOptions
      - remote: bool (default false for robustness)
      - include_provider: Vec<ProviderId>
      - refresh_cache: bool
  - v0 default:
      - static catalog first
      - optional remote fetch per provider when endpoint exists
      - runtime uses in-memory catalog
      - optional JSON export helper for pinning/versioning

  ## Internal Module Layout

  - src/core/
      - types.rs canonical domain types
      - traits.rs adapter/runtime contracts
      - error.rs runtime and provider error taxonomy
  - src/transport/
      - shared HTTP client wrapper (timeouts, retries, headers, tracing)
  - src/registry/
      - adapter registry, model routing, capability checks
  - src/providers/
      - openai/, anthropic/, openrouter/
      - request/response translators per provider
  - src/catalog/ (new)
      - static catalog, discovery aggregation, export
  - src/pricing/ (new)
      - pricing table lookup and cost computation
  - src/handoff/ (new)
      - cross-provider message normalization logic

  ## Provider Adapter Implementation Details (v0)

  ### OpenAI

  - Use Responses API JSON shape through the shared translator contract.
  - Adapter composes auth + transport + translator; translator owns protocol conversion.
  - Provider-specific field mapping details are deferred to provider implementation docs/tests.

  ### Anthropic

  - Use Messages API shape through the shared translator contract.
  - Adapter composes auth + transport + translator; translator owns protocol conversion.
  - Provider-specific field mapping details are deferred to provider implementation docs/tests.

  ### OpenRouter

  - Use OpenAI-compatible endpoint shape with compatibility flags through the shared translator contract.
  - Adapter composes auth + transport + translator; translator owns protocol conversion.
  - Provider-specific field mapping details are deferred to provider implementation docs/tests.

  ## Error Taxonomy

  - RuntimeError
      - ConfigError
      - CredentialMissing { provider, env_candidates }
      - RoutingError
      - CapabilityMismatch
      - TransportError
      - ProviderProtocolError
      - SerializationError
      - CostCalculationError
  - Include provider/model/request-id metadata where possible.

  ## Testing Plan

  ### Unit tests

  1. Canonical ↔ provider translation for each adapter (golden fixtures).
  2. Usage normalization with missing/partial token fields.
  3. Cost computation with exact/partial/missing pricing entries.
  4. Registry routing precedence and ambiguity handling.
  5. Handoff normalization:
      - same-provider thinking preserved
      - cross-provider thinking converted to <thinking> text
      - tool calls/results preserved

  ### Integration tests (mock HTTP)

  1. run() success for OpenAI/Anthropic/OpenRouter.
  2. Provider-specific error payload mapping to RuntimeError.
  3. Retry and timeout behavior.
  4. Structured output and tool-call responses roundtrip.

  ### Contract tests

  1. Canonical request fixture run through adapter encoder snapshots.
  2. Provider response fixture run through adapter decoder snapshots.
  3. Ensure no streaming assumptions in APIs.

  ### Acceptance criteria

  1. All v0 adapters pass canonical contract tests.
  2. ProviderRuntime::run() works end-to-end for 3 providers.
  3. Missing credentials produce actionable provider-specific errors.
  4. Usage and cost available when underlying data exists.
  5. Cross-provider handoff transformation deterministic and tested.
  6. No runtime-managed tool loop/session state.

  ## Documentation Deliverables

  1. README.md with quickstart for runtime builder + run().
  2. Provider config matrix for OpenAI/Anthropic/OpenRouter.
  3. Clear non-goals section (no streaming, no loop orchestration, no OAuth flows in v0).
  4. Example showing harness-managed tool loop using canonical types.

  ## Assumptions and Defaults

  - Default runtime call is non-streaming only.
  - Default discovery mode is static catalog; remote discovery is opt-in.
  - Default pricing source is local config table; provider-reported cost is consumed when available.
  - OAuth providers are deferred; token provider trait is included for forward compatibility.
  - Canonical layer unifies semantics, not raw JSON protocol shapes.
  - Translator contract is crate-private and not re-exported as public API.
  - Provider-specific mapping tables are intentionally deferred until provider implementation stages.
  - Runtime is async (tokio) and transport uses shared reqwest client.
