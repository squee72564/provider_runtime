# Stage 1 - Freeze Canonical Domain Types

Files:
- [src/core/types/mod.rs](src/core/types/mod.rs)

Goal:
Define the canonical provider-agnostic domain schema and serde behavior used by all later stages.

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

Field-Level Contract:
- `ProviderRequest`
  - `model: ModelRef`
  - `messages: Vec<Message>`
  - `tools: Vec<ToolDefinition>` default empty; omitted when empty
  - `tool_choice: ToolChoice` default `auto`
  - `response_format: ResponseFormat` default `text`
  - `temperature: Option<f32>` omitted when missing
  - `top_p: Option<f32>` omitted when missing
  - `max_output_tokens: Option<u32>` omitted when missing
  - `stop: Vec<String>` default empty; omitted when empty
  - `metadata: BTreeMap<String, String>` default empty; omitted when empty
- `ProviderResponse`
  - `output: AssistantOutput`
  - `usage: Usage`
  - `cost: Option<CostBreakdown>` omitted when missing
  - `provider: ProviderId`
  - `model: String`
  - `raw_provider_response: Option<serde_json::Value>` omitted when missing
  - `finish_reason: FinishReason`
  - `warnings: Vec<RuntimeWarning>` default empty; omitted when empty
- `ModelRef`
  - `provider_hint: Option<ProviderId>` omitted when missing
  - `model_id: String`
- `Message`
  - `role: MessageRole`
  - `content: Vec<ContentPart>`
- `ToolDefinition`
  - `name: String`
  - `description: Option<String>` omitted when missing
  - `parameters_schema: serde_json::Value`
- `ToolCall`
  - `id: String`
  - `name: String`
  - `arguments_json: serde_json::Value`
- `ToolResult`
  - `tool_call_id: String`
  - `content: Vec<ContentPart>`
- `AssistantOutput`
  - `content: Vec<ContentPart>`
  - `structured_output: Option<serde_json::Value>` omitted when missing
- `Usage`
  - `input_tokens: Option<u64>`
  - `output_tokens: Option<u64>`
  - `reasoning_tokens: Option<u64>`
  - `cached_input_tokens: Option<u64>`
  - `total_tokens: Option<u64>`
- `CostBreakdown`
  - `currency: String`
  - `input_cost: f64`
  - `output_cost: f64`
  - `reasoning_cost: Option<f64>` omitted when missing
  - `total_cost: f64`
  - `pricing_source: PricingSource`
- `RuntimeWarning`
  - `code: String`
  - `message: String`
- `ModelInfo`
  - `provider: ProviderId`
  - `model_id: String`
  - `display_name: Option<String>` omitted when missing
  - `context_window: Option<u32>` omitted when missing
  - `max_output_tokens: Option<u32>` omitted when missing
  - `supports_tools: bool`
  - `supports_structured_output: bool`
- `ModelCatalog`
  - `models: Vec<ModelInfo>` default empty; omitted when empty
- `DiscoveryOptions`
  - `remote: bool`
  - `include_provider: Vec<ProviderId>` default empty; omitted when empty
  - `refresh_cache: bool`
- `ProviderCapabilities`
  - `supports_tools: bool`
  - `supports_structured_output: bool`
  - `supports_thinking: bool`
  - `supports_remote_discovery: bool`
- `AdapterContext`
  - Stage 1 placeholder struct with deterministic serde and no transport/auth fields

Serde / Normalization Policy:
- All structs derive serde and use `#[serde(deny_unknown_fields)]`.
- Optional fields are omitted when `None`.
- Selected collections are omitted when empty as documented above.
- Enum wire names use `snake_case`.
- All enums use internal tagging with discriminator key `type`.
- No provider-specific protocol fields are allowed except `raw_provider_response` in `ProviderResponse`.

Deterministic Behavior:
- `Usage::derived_total_tokens()` returns:
  - explicit `total_tokens` when present
  - otherwise `input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0)`
- `reasoning_tokens` and `cached_input_tokens` do not alter derived total in v0.

Internal Responsibilities:
- Serde derives and canonical field normalization.
- Deterministic defaults for optional fields.
- No provider-specific JSON fields.

Unit Tests:
- `test_provider_request_serde_roundtrip`
- `test_usage_total_tokens_derivation`
- `test_content_part_invariants`

Acceptance Criteria:
- Canonical types compile and are provider-agnostic.
- No streaming/session/orchestration fields introduced.
- `cargo check --lib` passes.
