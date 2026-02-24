# Stage 18/19 Deep Research - OpenRouter Chat Completions Surface

## Scope
This document defines the OpenRouter adapter/translator surface for `provider-runtime` v0 with the following boundaries:
- Endpoint: `POST /api/v1/chat/completions`
- Mode: non-streaming only
- Canonical API remains provider-agnostic and unchanged
- OpenRouter-specific controls are adapter-private (`OpenRouterAdapterOptions`)

## Canonical Boundary
Canonical request/response semantics remain defined by `ProviderRequest` and `ProviderResponse` in `src/core/types/mod.rs`.

Design constraints:
- No provider wire shapes in canonical types
- No OpenRouter routing/plugin objects in canonical metadata
- Deterministic encode/decode behavior
- Deterministic error behavior for unsupported intent

## Translator Contract Alignment
The OpenRouter translator follows crate-private `ProviderTranslator`:
- `encode_request(&ProviderRequest) -> OpenRouterEncodedRequest`
- `decode_response(&OpenRouterDecodeEnvelope) -> ProviderResponse`

Responsibilities:
- Canonical -> OpenRouter payload mapping
- OpenRouter payload -> canonical normalization
- Deterministic protocol validation errors
- Stable warnings for allowed partial/lossy cases

## Request Mapping Surface
### Canonical fields
- `model.model_id` -> `model` (or `models` when fallback list provided)
- `messages` -> `messages`
- `tools` -> `tools`
- `tool_choice` -> `tool_choice`
- `response_format` -> `response_format`
- `temperature` -> `temperature`
- `top_p` -> `top_p`
- `max_output_tokens` -> `max_completion_tokens`
- `stop` -> `stop`
- `metadata` -> `metadata`
- force `stream=false`

### Adapter-private OpenRouter fields
- `fallback_models` -> `models`
- `provider_preferences` -> `provider`
- `plugins` -> `plugins`
- `parallel_tool_calls` -> `parallel_tool_calls`
- `frequency_penalty` -> `frequency_penalty`
- `presence_penalty` -> `presence_penalty`
- `logit_bias` -> `logit_bias`
- `logprobs` -> `logprobs`
- `top_logprobs` -> `top_logprobs`
- `reasoning` -> `reasoning`
- `seed` -> `seed`
- `user` -> `user`
- `session_id` -> `session_id`
- `trace` -> `trace`
- deprecated pass-through compatibility:
  - `route` -> `route`
  - `max_tokens` -> `max_tokens`

## Strict Validation Rules
### Canonical input validation
- `provider_hint` must be `Openrouter` when present
- `model_id` must be non-empty
- `stop` length <= 4
- `metadata` <= 16 pairs, key <= 64 chars, value <= 512 chars
- `temperature` in [0, 2]
- `top_p` in [0, 1]
- `max_output_tokens >= 1` when present

### Adapter-private option validation
- `frequency_penalty`, `presence_penalty` in [-2, 2]
- `top_logprobs` in [0, 20]
- `logit_bias` object with numeric values
- `reasoning` object
- `trace` object
- `user` non-empty when provided
- `session_id` non-empty and <= 128 chars
- `route` in {`fallback`, `sort`} when provided
- `max_tokens >= 1` when provided

### Unsupported-mode policy (hard errors)
- Any non-text modality in `modalities`
- `image_config` (multimodal/image-generation scope)
- `debug` (streaming-focused debug surface)
- `stream_options` (streaming surface)

## Tool Semantics
- Tool definition names must match `^[A-Za-z0-9_-]{1,64}$`
- Tool parameters schema must be a JSON object
- `ToolChoice::Specific` must reference a declared tool
- Assistant tool calls encode arguments as deterministic stable JSON strings
- Tool result messages require exactly one `ToolResult` content part
- Tool-role messages require tools to be declared for workflow consistency

## Decode Normalization
- Top-level `error` payload -> protocol error
- `choices` required and non-empty
- `choices[0].message.role` must be `assistant` when present
- `finish_reason=error` -> protocol error
- `finish_reason` mapping:
  - `stop` -> `Stop`
  - `length` -> `Length`
  - `tool_calls` -> `ToolCalls`
  - `content_filter` -> `ContentFilter`
  - unknown -> `Other` + warning

Message decoding:
- `content: string` -> canonical `Text`
- `content: null` -> no text part
- `content: array` -> only `type=text` items supported; non-text item type is protocol error
- `refusal` string is preserved as canonical `Text`
- `tool_calls`:
  - must be array
  - each call requires `id`, `type`, `function`
  - `type` must be `function`
  - `function.arguments` parsed as JSON when possible, else kept as raw string with warning
- `reasoning` and `reasoning_details` are mapped to canonical `Thinking` with provider hint

Usage decoding:
- Map `prompt_tokens`, `completion_tokens`, `total_tokens`
- Map optional `prompt_tokens_details.cached_tokens`
- Map optional `completion_tokens_details.reasoning_tokens`
- Missing/null usage yields warning and default empty usage
- Partial usage yields warning

Structured output:
- Only attempted when requested response format is non-text
- Parsed from joined text blocks
- parse failure -> warning, `structured_output=None`

## Adapter Boundary (Stage 19)
Adapter responsibilities stay orchestration-only:
- API key resolution precedence (constructor key, context metadata, env var)
- HTTP request execution via transport
- Request header injection (auth, attribution)
- Transport error normalization and OpenRouter error-envelope formatting
- Translator invocation for all protocol mapping

## Non-Goals (v0)
- Streaming unification
- Multimodal request/response canonicalization
- Session state orchestration
- Provider-specific metadata leakage into canonical response fields

## Verification Checklist
- OpenRouter translator category tests pass
- OpenRouter adapter tests pass
- Full cargo gates pass:
  - `cargo check --all-targets`
  - `cargo fmt --all --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --all-targets --all-features`
