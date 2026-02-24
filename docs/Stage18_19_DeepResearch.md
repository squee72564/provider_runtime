hat Completions schema**, **parameters**, **tool calling**, **structured outputs**, **routing docs**, **error semantics**, plus **OpenAPI JSON** for schema edge details. citeturn24view0turn4view1turn6view0turn7view5turn12view0turn11view3turn15view1turn27view0  # OpenRouter Chat Completions Adapter Surface Mapping for provider-runtime

I’m putting on my “API adapter + wire-protocol mapping” specialist hat for this one.

The scope here is **OpenRouter’s OpenAI-compatible Chat Completions endpoint** (`POST /api/v1/chat/completions`) in **non-streaming mode only**, including OpenRouter routing extensions (model fallbacks + provider preferences) but **without letting router/provider transport details leak into canonical**. citeturn4view1turn24view0turn5view3turn21view3

Research steps I followed (so you can audit the derivations):
1) Pulled your **canonical types** and **translator contract** from the repo. citeturn0view0turn0view1turn21view1turn21view3  
2) Cross-referenced OpenRouter’s **API Overvie (enumerated fully):**
- Associated types:
  - `type RequestPayload;`
  - `type ResponsePayload;` citeturn0view0
- Methods:
  - `fn encode_request(&self, req: &ProviderRequest) -> Result<RequestPayload, ProviderError>`
  - `fn decode_response(&self, payload: &Self::ResponsePayload) -> Result<ProviderResponse, ProviderError>` citeturn0view0

This aligns with the Stage 18 mandate: “canonical-to-provider and provider-to-canonical translation,” deterministic, no silent lossy conversion, and protocol isolation. citeturn5view1turn5view3

#### A.1 encode_request responsibilities, invariants, guarantees

**Responsibilities**
- Convert a canonical `ProviderRequest` into an OpenRouter `/api/v1/chat/completions` JSON body (non-streaming). citeturn0view0turn0view1turn4view1turn24view0turn5view1
- Enforce validation rules that prevent ambiguous or lossy mapping.
- Encode tool definitions and tool-choice semantics in OpenRouter’s OpenAI-compatible shape. citeturn6view0turn24view2turn27view0
- Encode structured output controls via `response_format`. citeturn3view8turn12view0turn29view0turn26view2
- Ensure deterministic serialization (same canonical request ⇒ identical provider payload) per Stage 18. citeturn5view1turn5view3

**Input invariants (must validate)**
- Canonical request fields are the only source of semantic intent; do not manufacture provider-only knobs from canonical metadata. (This is a “no leakage” boundary requirement from v0 plan + Stage 19.) citeturn5view3turn5view2turn0vi
3) Designed deterministic encode/decode algorithms that fit `translator_contract.rs` exactly, identified what must sit in adapter-private config, and called out canonical/contract gaps explicitly. citeturn0view0turn5view1turn5view2turn25view3  

## Translator contract alignment

### A. Translator Contract Alignment

Your crate-private translator contract is extremely small: two associated payload types + two methods. citeturn0view0turn5view1turn5view3  

**Contract surfaceew1
- Message sequences that include tools must obey OpenRouter’s schema expectations (see tool calling section below), in particular: tool schemas must be present when doing tool workflows. citeturn3view0turn7view5turn8view0

**Output guarantees**
- Output is a single JSON object compatible with OpenRouter’s chat completions request shape. citeturn4view1turn29view0turn26view2  
- No streaming fields enabled (adapter will set/force `stream=false`; translator must not emit streaming-only knobs). citeturn9view0turn5view3turn15view6

**Error conditions**
- Return `ProviderError::Serialization` when canonical values cannot be deterministically encoded (example: cannot serialize tool arguments to stable JSON string). citeturn21view1turn5view1
- Return `ProviderError::Protocol` for invalid canonical states with respect to OpenRouter semantics (example: tool messages present but tools list absent). citeturn21view1turn5view1turn3view0

**Must not leak**
- No OpenRouter routing objects (`provider`, `models`, `route`, `transforms`, `plugins`) may be inferred from canonical fields and placed into a canonical-visible interface; if used, they must be adapter-private configuration only. citeturn24view1turn29view0turn5view2turn5view3

#### A.2 decode_response responsibilities, invariants, guarantees

**Responsibilities**
- Convert OpenRouter’s non-streaming completion response into canonical `ProviderResponse`, including:
  - text output,
  - tool call requests (`tool_calls`),
  - usage accounting,
  - finish reason normalization. citeturn25view4turn25view3turn25view0turn0view1
- Detect and fail on error bodies, including the “HTTP 200 with error embedded in body” cases (see error handling section). citeturn15view4turn3view6turn25view1

**Input invariants**
- Payload must have the non-streaming shape (choices contain `message`, not `delta`). citeturn25view4  
- If the payload corresponds to an error response, it must be mapped to a `ProviderError` instead of returning “successful” canonical data. citeturn15view1turn15view4turn25view1

**Output guarantees**
- Deterministic decoding: same payload ⇒ same canonical output. citeturn5view1turn5view3
- Canonical `provider` must be `ProviderId::Openrouter`. citeturn0view1

**Error conditions**
- `ProviderError::Protocol` for malformed provider payloads (missing required response shape) or embedded provider-side errors in a 200 response body. citeturn21view1turn15view4turn25view1
- `ProviderError::Serialization` for JSON type mismatches that prevent canonicalization (e.g., tool call arguments not a string). citeturn21view1turn25view2

**Must not leak**
- Do not surface OpenRouter’s `native_finish_reason` or router/provider metadata directly into canonical fields; treat it as debug-only (raw response) or drop it. OpenRouter explicitly notes `native_finish_reason` can include provider-specific values. citeturn25view0turn25view2  
- Do not include provider names from error metadata (`provider_name`) in canonical error messages. OpenRouter error metadata schemas include provider-specific details. citeturn15view2turn25view2turn21view1

#### A.3 Are canonical changes required?

**No hard requirement** to satisfy the current translator trait shape for basic text + tools + basic structured output hints. The canonical request already includes the minimum knobs expected in Stage 18. citeturn0view0turn0view1turn5view1  

However, **there is a real “semantic completeness” gap** for structured output verification because `decode_response` has no access to the request’s `ResponseFormat` or JSON Schema (details in the “Canonical gaps” section). citeturn0view0turn0view1turn12view4  

## Canonical to OpenRouter request mapping

### B. Canonical → OpenRouter Request Mapping

OpenRouter describes the request schema as OpenAI-chat-like, plus OpenRouter-only routing/plugin knobs. citeturn24view1turn29view0turn26view2  

#### B.1 Field-by-field mapping table

| Canonical field (`ProviderRequest`) | OpenRouter field | Transformation rules | Validation logic | Invalid states / edge cases |
|---|---|---|---|---|
| `model: ModelRef { model_id, provider_hint }` citeturn0view1 | `model` **or** `models[]` citeturn29view0turn3view4 | Default: `model = req.model.model_id`. If adapter-private fallback list exists, prefer `models = [primary] + fallbacks` and omit `model` to avoid ambiguity. citeturn3view4turn29view0 | Ensure primary model id is non-empty. If using fallbacks, ensure `models` list order is deterministic (primary first). citeturn3view4turn5view1 | Router can return a different `response.model` if fallback triggers. Canonical must tolerate this and only surface final `ProviderResponse.model`. citeturn3view4turn25view2 |
| `messages: Vec<Message>` citeturn0view1 | `messages[]` citeturn4view1turn29view0turn26view2 | Map each canonical message to a single OpenRouter message object (role + content and optional tool fields). | Must have ≥1 message for chat completions (OpenAPI requires minItems 1). citeturn26view2 | Tool workflows require special rules: tool calls are on assistant messages; tool results are role `"tool"`. citeturn24view2turn7view1 |
| `Message.role = System` citeturn0view1 | `role:"system"` message citeturn4view1turn29view0 | Encode as `{role:"system", content:<string>}`. OpenRouter uses system as a normal message, not a top-level parameter. citeturn4view1turn29view0 | Content must be representable as a string (see ContentPart rules below). | If canonical system message contains tool parts or thinking parts, treat as protocol error (unsupported / ambiguous). citeturn0view1turn5view1 |
| `Message.role = User` citeturn0view1 | `role:"user"` message citeturn29view0turn24view2 | Encode as `{role:"user", content:<string>}`. OpenRouter supports `content` as string or ContentPart[] (images) but canonical doesn’t support images, so stick to string. citeturn24view2turn29view0 | Ensure canonical content parts can be rendered to string deterministically. | If user message contains ToolCall/ToolResult parts, treat as protocol error. citeturn0view1turn24view2 |
| `Message.role = Assistant` citeturn0view1 | `role:"assistant"` message citeturn29view0turn25view1 | If assistant message includes canonical ToolCalls, encode them into message-level `tool_calls[]`. Text parts become `content` (string). If no text parts and tool_calls present, emit `content:null` (matches OpenRouter example). citeturn7view0turn25view1 | ToolCalls must encode `arguments` as JSON string. Validate JSON value serializable deterministically. citeturn7view0turn25view2turn5view1 | Mixed text + tool_calls is allowed but ordering is ambiguous; enforce stable canonical ordering (see response mapping). citeturn25view1turn5view1 |
| `Message.role = Tool` citeturn0view1 | `role:"tool"` message with `tool_call_id` citeturn24view2turn7view1 | Encode tool result messages as `{role:"tool", tool_call_id, content:<string>}`. (Optional `name` exists in OpenRouter schema but canonical has no slot; drop.) citeturn24view2 | Ensure exactly one tool_call_id per tool result (canonical has it). Ensure tool result content representable as string. citeturn0view1turn24view2 | If tool_call_id empty, or tool message content can’t be represented deterministically, protocol/serialization error. |
| `tools: Vec<ToolDefinition>` citeturn0view1 | `tools[]` “OpenAI tool calling shape” citeturn6view0turn29view0turn26view2 | Each canonical tool becomes: `{type:"function", function:{name, description?, parameters:<schema>}}`. OpenAPI enforces `name` length and constraints; treat canonical tool name violations as encoding errors. citeturn27view0turn29view0 | Validate `name` length ≤64 and conforms to allowed chars as described in OpenAPI schema. Validate `parameters_schema` is an object. citeturn27view0turn0view1 | Tools must be included in *every* tool-workflow request so router can validate schemas on each call (OpenRouter explicitly warns). citeturn3view0turn8view0 |
| `tool_choice: ToolChoice` citeturn0view1 | `tool_choice` citeturn6view0turn29view0turn27view0 | If `tools` empty: omit `tool_choice`. If `tools` non-empty: `None→"none"`, `Auto→"auto"`, `Required→"required"`, `Specific{name}→{type:"function", function:{name}}`. citeturn6view0turn27view0turn29view0 | If `Specific{name}` refers to a non-existent tool in `tools`, protocol error (don’t auto-downgrade silently). citeturn5view1turn0view1 | Canonical default is Auto. With tools empty, treated as “no tool calling available” (omit). citeturn0view1turn24view1 |
| `response_format: ResponseFormat` citeturn0view1 | `response_format` citeturn3view8turn29view0turn26view2 | `Text`: omit `response_format`. `JsonObject`: `{type:"json_object"}`. `JsonSchema{name,schema}`: `{type:"json_schema", json_schema:{name, strict:true, schema}}`. OpenRouter supports both JSON mode and JSON schema mode. citeturn29view0turn12view0turn3view8 | Validate schema is object for JsonSchema. If invalid JSON Schema, OpenRouter returns an error. citeturn12view4 | Canonical has no “strict” toggle; mapping chooses strict=true by default in schema mode per OpenRouter best-practice guidance. citeturn12view1turn29view0 |
| `temperature: Option<f64>` citeturn0view1 | `temperature` citeturn6view6turn26view2 | Pass through when Some. | Must be in [0,2]. citeturn6view6turn26view2 | Out-of-range: protocol error. |
| `top_p: Option<f64>` citeturn0view1 | `top_p` citeturn6view7turn26view2 | Pass through when Some. | Must be in [0,1]. citeturn6view7turn26view2 | Out-of-range: protocol error. |
| `max_output_tokens: Option<u64>` citeturn0view1 | `max_completion_tokens` citeturn9view0turn26view2 | If Some(n): set `max_completion_tokens=n`. Do **not** use deprecated `max_tokens`. OpenRouter docs explicitly mark max_tokens deprecated in favor of max_completion_tokens. citeturn9view0turn4view3turn26view2 | Must be ≥1. citeturn9view0turn26view2 | If set to 0: protocol error. |
| `stop: Vec<String>` citeturn0view1 | `stop` citeturn9view0turn3view8turn26view2 | If empty: omit. If length=1: can send string or array; prefer array for determinism. If 2–4: send array. | OpenRouter caps stop array at 4. citeturn9view0turn26view2 | If >4: protocol error (don’t silently truncate). |
| `metadata: BTreeMap<String,String>` citeturn0view1 | `metadata` citeturn9view0turn26view2 | Pass through as JSON map. | OpenRouter caps metadata at max 16 pairs and size limits per key/value. citeturn9view0turn26view2 | If violation: protocol error (recommended) to avoid silent truncation; alternative is deterministic truncation with warning (if you choose a “lenient mode”). citeturn5view1 |

#### B.2 ContentPart → OpenRouter message.content mapping rules

Canonical `Message.content` is `Vec<ContentPart>` where parts can be Text, Thinking, ToolCall, ToolResult. citeturn0view1turn5view3  

OpenRouter’s message schema (chat completions) expects:
- for user/system/assistant: `content: string | ContentPart[]` (with ContentPart[] explicitly aimed at user role for multimodal), and
- for tool messages: `content: string` plus `tool_call_id`. citeturn24view2turn29view0turn26view2  

**Safe mapping rules (non-streaming, no multimodal):**
- `ContentPart::Text {text}`: append to a buffer; final `content` string is a deterministic concatenation of all text parts in order (recommend joining with `"\n"` between parts to preserve boundaries). citeturn0view1turn5view1turn29view0
- `ContentPart::ToolCall {tool_call}`:
  - Only valid inside an Assistant-role message.
  - Map into `message.tool_calls[]` entries and do **not** attempt to place tool-call JSON into text content. citeturn0view1turn25view1turn7view0
- `ContentPart::ToolResult {tool_result}`:
  - Only valid inside a Tool-role message.
  - Map into `{role:"tool", tool_call_id, content:<string>}`. citeturn0view1turn24view2turn7view1
- `ContentPart::Thinking {..}`:
  - **No OpenRouter chat-completions field exists that can carry this canonically.**
  - Treat as unsupported in encode (protocol error) unless your runtime formally defines a canonical “thinking-to-text escaping” rule at the canonical layer (but Stage 18’s “no silent lossy conversion” makes “just jam it into text” a bad default). citeturn0view1turn5view1turn5view3

#### B.3 Tool definition mapping → `tools[]`

OpenRouter describes tool calling as OpenAI tool calling shape, and the OpenAPI spec makes it explicit. citeturn6view0turn29view0turn27view0  

Canonical:
```text
ToolDefinition { name, description?, parameters_schema: JSON }
``` citeturn0view1  

OpenRouter:
```json
{ "type": "function", "function": { "name": "...", "description": "...", "parameters": { ... } } }
``` citeturn29view0turn7view5turn27view0  

**Validation hazards**
- OpenAPI specifies constraints on tool function name length and allowed chars; fail encode if canonical tool names violate these to avoid upstream schema rejection. citeturn27view0turn5view1
- `parameters_schema` must be a JSON object representing JSON Schema. citeturn0view1turn29view2turn27view0

#### B.4 Tool choice mapping → `tool_choice`

OpenRouter supports:
- `"none"`, `"auto"`, `"required"`, or named tool choice object. citeturn6view0turn27view0turn29view0  

Canonical maps 1:1 onto those values. citeturn0view1turn27view0  

**Invariant**
- `ToolChoice::Specific {name}` must reference a tool that exists in `tools[]`; do not downgrade to `"auto"` without an explicit warning path, and prefer deterministic hard error. citeturn5view1turn6view0

#### B.5 Routing controls mapping (adapter-private)

Canonical has no routing knobs beyond `model_id`. citeturn0view1turn5view1  

OpenRouter routing knobs live in request-level OpenRouter-only fields:
- `models?: string[]` (model fallbacks) citeturn29view0turn3view4
- `provider?: ProviderPreferences` (provider selection/sorting/filters) citeturn29view0turn10view2turn11view4
- `plugins?: Plugin[]` (including response-healing) citeturn29view0turn12view4

**Design requirement (no leakage):**
- These must be represented as **adapter-private configuration**, not as canonical fields or canonical metadata keys. citeturn5view2turn5view3turn24view1  

A safe adapter-private config surface (conceptual, not code) that doesn’t touch canonical:
- `fallback_models: Option<Vec<String>>` → request `models[]`. citeturn3view4turn29view0  
- `provider_preferences: Option<ProviderPreferences>` → request `provider`. Include controls like:
  - `order` (explicit provider slug order) citeturn10view2turn11view1
  - `allow_fallbacks` / `allowFallbacks` (disable provider fallback) citeturn10view0turn11view2
  - `require_parameters` (only route to providers that support all request parameters) citeturn10view1turn11view3
  - `data_collection` allow/deny, `zdr`, `enforce_distillable_text` (data policy filters) citeturn11view4turn11view5
  - `only`, `ignore` (allowlist/denylist providers) citeturn11view6turn11view7
  - `sort` / advanced sort partitioning (affects determinism/availability tradeoffs) citeturn10view3turn11view1  
- `plugins: Option<Vec<PluginSpec>>` → request `plugins[]`, including `response-healing` for json_schema robustness (non-streaming). citeturn29view2turn12view4  
- `parallel_tool_calls: Option<bool>` (canonical lacks it) → request `parallel_tool_calls`. citeturn6view0turn7view4turn26view2

#### B.6 Determinism and routing variability

OpenRouter routing features can change which upstream model/provider responds (and may affect determinism even with `seed`). OpenRouter notes deterministic sampling via `seed` is not guaranteed for some models. citeturn6view5turn29view0turn3view4  

**Deterministic contract stance**
- The translator must be deterministic **given the canonical input**, but it cannot force OpenRouter to be deterministic when routing/fallbacks/providers vary. Stage 18 only requires deterministic translation (same input ⇒ same encoded JSON), not deterministic model behavior. citeturn5view1turn3view4turn10view3  

## OpenRouter response to canonical mapping

### C. OpenRouter Response → Canonical Mapping

OpenRouter states it normalizes responses to comply with the OpenAI Chat API and always returns a `choices` array; non-streaming uses `message` objects. citeturn25view4turn25view1turn23search2  

#### C.1 Deterministic response parsing algorithm (non-streaming)

**Inputs**
- `payload`: decoded JSON body from OpenRouter `/api/v1/chat/completions`. citeturn4view1turn25view4

**Algorithm (pseudocode, not implementation code)**

```text
function decode_openrouter_chat_completion(payload):
  # 0) Error envelope detection (HTTP 4xx/5xx body OR 200-with-error-body)
  if payload has top-level key "error" with {code, message, metadata?}:
      return ProviderError::Status or ::CredentialsRejected depending on code/status
      (adapter usually handles HTTP status; translator treats as Protocol if status is unavailable)

  # 1) Structural validation
  require payload.object == "chat.completion" OR allow missing (OpenRouter schemas vary)
  require payload.choices is array and len >= 1

  # 2) Select choice[0] deterministically
  choice = payload.choices[0]

  # 3) Embedded error detection (200 OK but error in body)
  if choice.error exists OR choice.finish_reason == "error":
      return ProviderError::Protocol(message = sanitized(choice.error or "finish_reason=error"))

  # 4) Map finish_reason
  finish = map_finish_reason(choice.finish_reason)

  # 5) Message extraction
  msg = choice.message
  role = msg.role
  content = msg.content (string or null)
  tool_calls = msg.tool_calls? (array)

  out_parts = []
  if content is non-empty string:
      out_parts.push(Text(content))

  if tool_calls exists:
      for each tc in tool_calls:
          tool_call = parse_tool_call(tc)  # id, function.name, function.arguments string->JSON value
          out_parts.push(ToolCall(tool_call))

  # 6) Usage mapping (optional but expected for non-streaming)
  usage = map_usage(payload.usage)

  # 7) Construct canonical ProviderResponse
  return ProviderResponse {
      output: { content: out_parts, structured_output: maybe_parse_json(content) },
      usage: usage,
      cost: maybe_map_cost(payload.usage),
      provider: ProviderId::Openrouter,
      model: payload.model (string),
      raw_provider_response: None (unless debug opt-in),
      finish_reason: finish,
      warnings: collected_warnings
  }
```

Key rule sources used in this algorithm:
- finish_reason normalization and `native_finish_reason` existence (and why it must be treated as non-canonical). citeturn25view0turn25view2  
- non-streaming choice shape includes `message.content` and optional `tool_calls`, plus optional `error`. citeturn25view1turn25view2  
- OpenRouter chat completions can embed errors in a 200 response once generation has started. citeturn15view4turn3view6  

#### C.2 Tool calls in responses → canonical ToolCall

OpenRouter response messages can include `tool_calls?: ToolCall[]` in non-streaming responses. citeturn25view1turn25view2  

Tool call item schema is consistent with OpenAI tool calls:
- `id: string`
- `type: "function"`
- `function: { name: string, arguments: string }` citeturn7view0turn25view2  

Canonical tool call type:
- `ToolCall { id, name, arguments_json: Value }` citeturn0view1  

**Deterministic mapping**
- `canonical.id = tool_calls[i].id` (no remapping; preserves linkage with `tool_call_id`). citeturn7view1turn24view2  
- `canonical.name = tool_calls[i].function.name` citeturn7view0turn25view2  
- `canonical.arguments_json`:
  - Attempt to parse `function.arguments` as JSON (it is a JSON string in OpenRouter examples). citeturn7view0turn8view0  
  - If parsing fails, store raw string in `arguments_json` (as JSON string) and emit a canonical warning (provider-neutral). This preserves data without pretending it’s valid JSON. citeturn0view1turn5view1  

#### C.3 Usage mapping

OpenRouter’s overview states:
- Usage data is returned for non-streaming.
- OpenRouter returns detailed usage breakdown including `prompt_tokens_details.cached_tokens` and `completion_tokens_details.reasoning_tokens`. citeturn25view3  

Canonical `Usage` fields:
- `input_tokens`, `output_tokens`, `reasoning_tokens`, `cached_input_tokens`, `total_tokens`. citeturn0view1  

**Mapping**
- `input_tokens = usage.prompt_tokens`
- `output_tokens = usage.completion_tokens`
- `total_tokens = usage.total_tokens`
- `cached_input_tokens = usage.prompt_tokens_details.cached_tokens` (if present)
- `reasoning_tokens = usage.completion_tokens_details.reasoning_tokens` (if present) citeturn25view3turn0view1  

**Fallback behavior**
- If `usage` missing or null (schema allows it), leave canonical `Usage` as `Usage::default()` with all `None`, and emit a warning like `usage.missing`. citeturn0view1turn9view3turn25view3  

#### C.4 Model identity and fallbacks

OpenRouter can fail over a request to fallback models when using `models[]`. The docs explicitly say it will try fallback models if the primary errors (rate limits, moderation flags, downtime, etc.). citeturn3view4  

In responses:
- `payload.model` indicates the model used for completion (and can differ from the originally requested one). citeturn25view2turn9view3  

Canonical policy:
- Store only the **actual** model returned in `ProviderResponse.model`.
- Do not surface the fallback list, provider ordering, or routing decision trail in canonical fields. citeturn0view1turn5view2turn3view4  

### E. Finish Reason Mapping Matrix

OpenRouter normalizes `finish_reason` to exactly:
- `tool_calls`, `stop`, `length`, `content_filter`, `error`. citeturn25view0  

Canonical `FinishReason`:
- `Stop`, `Length`, `ToolCalls`, `ContentFilter`, `Error`, `Other`. citeturn0view1  

| OpenRouter `finish_reason` | Canonical `FinishReason` | Notes / invariants |
|---|---|---|
| `stop` citeturn25view0 | `Stop` citeturn0view1 | Normal completion. |
| `length` citeturn25view0 | `Length` citeturn0view1 | Token limit / truncation completion. |
| `tool_calls` citeturn25view0turn7view0 | `ToolCalls` citeturn0view1 | Indicates `message.tool_calls` should be present. If absent, emit warning. citeturn25view1turn5view1 |
| `content_filter` citeturn25view0 | `ContentFilter` citeturn0view1 | May correspond to moderation/refusal behavior. Keep provider-neutral. citeturn15view2turn25view0 |
| `error` citeturn25view0turn15view4 | `Error` citeturn0view1 | **Do not return ProviderResponse** if an error object exists or choice.error exists; instead return `ProviderError`. citeturn25view1turn15view4 |
| any other string or null citeturn25view1turn25view0 | `Other` citeturn0view1 | Do not surface raw provider enums. Optionally warn. citeturn25view0turn5view1 |

### G. Usage Mapping and extra usage details

OpenRouter’s detailed usage includes optional cost and cost_details, BYOK flags, server tool usage, etc. citeturn25view3  

Canonical policy (to avoid leakage):
- Map only the canonical fields that exist (`input_tokens`, `output_tokens`, `reasoning_tokens`, `cached_input_tokens`, `total_tokens`). citeturn0view1turn25view3  
- Everything else (BYOK flags, server tool usage) is **dropped** unless you are populating `raw_provider_response` in a debug-only mode. citeturn25view3turn5view3  

## Structured output handling

### D. Structured Output Handling

OpenRouter supports `response_format` with two JSON-oriented modes:
- `{ "type": "json_object" }` (JSON mode; guarantees valid JSON output)
- `{ "type": "json_schema", "json_schema": { name, strict?, schema } }` (schema mode). citeturn3view8turn29view0turn12view0  

#### D.1 Strategy one: JSON-only output via JSON mode

**Request mapping**
- Canonical `ResponseFormat::JsonObject` → `response_format: { type: "json_object" }`. citeturn0view1turn3view8turn29view0  
- OpenRouter warns: even in JSON mode, you should also explicitly instruct the model to produce JSON via system/user message. citeturn3view8  

**Extraction + validation rules**
- If assistant `message.content` is a string:
  - Parse as JSON (must succeed per OpenRouter JSON mode claim). citeturn3view8  
  - If parse succeeds: set `AssistantOutput.structured_output = Some(parsed_json)` and keep `AssistantOutput.content` with the raw text as `ContentPart::Text`. This preserves both representations. citeturn0view1turn3view8  
  - If parse fails: keep raw text in `content`, set `structured_output=None`, and emit a warning like `structured_output.json_parse_failed`. citeturn5view1turn0view1  

**Malformatted JSON handling**
- Never silently “repair” or auto-correct JSON at the translator layer (that would be provider-ish and non-deterministic).
- If repair is desired, it must be done by OpenRouter’s response-healing plugin (adapter-private) or by the client harness explicitly. citeturn12view4turn29view2  

#### D.2 Strategy two: JSON Schema–guided output via response_format json_schema

**Request mapping**
- Canonical `ResponseFormat::JsonSchema { name, schema }` →  
  `response_format: { type:"json_schema", json_schema:{ name:<name>, strict:true, schema:<schema> }}`.  
  OpenRouter’s structured output guide explicitly recommends always using strict mode (`strict: true`). citeturn0view1turn12view1turn29view0  

**Failure modes**
OpenRouter documents two explicit failure scenarios:
1) If the model doesn’t support structured outputs, the request fails with an error indicating lack of support.
2) If your JSON Schema is invalid, it returns an error. citeturn12view4  

**Adapter-private mitigation: Response Healing plugin**
- OpenRouter says: for non-streaming requests using `response_format` with `type:"json_schema"`, you can enable the Response Healing plugin to reduce invalid JSON risk. citeturn12view4turn29view2  
- Plugin schema in the overview: `{ id: string, enabled?: boolean, ... }`, and `response-healing` is named as an available plugin. citeturn29view0turn29view2  
- Since the plugin documentation is outside the allowed references list, treat plugin-specific options as **opaque passthrough** (adapter-private config) and do not enshrine them into canonical. citeturn29view2turn5view2  

**Extraction + schema validation**
- Parse `message.content` as JSON and set `structured_output` if parse succeeds.
- Schema validation of the output against the provided schema is desirable, but cannot be done inside the current translator trait because `decode_response` has no access to `ProviderRequest.response_format.schema`. citeturn0view0turn0view1turn12view0  
- Therefore, within current contract boundaries: you can **only parse**, not validate against the schema.

#### D.3 Detecting “requested structured output” vs incidental JSON

This is the biggest semantic wrinkle with the current contract:

- The response does not (in the documented schema) include an explicit “structured output requested” marker, and the translator’s `decode_response` has no request context. citeturn25view4turn0view0  
- So a translator can only:
  - Parse opportunistically (if content parses as JSON, set `structured_output`), or
  - Never parse (leave `structured_output=None` always), which defeats structured output support.

**Safe fallback behavior under current constraints**
- Always attempt to parse JSON when `message.content` appears to be JSON (object/array), set `structured_output` on success, otherwise leave it `None` and preserve raw text. This avoids false guarantees. citeturn0view1turn3view8turn12view4  

**Canonical/contract change needed for strict correctness**
- If you want strict “requested vs incidental” detection and schema validation, you need the request context during decode (see gap analysis). citeturn0view0turn0view1turn12view4  

## Tool calling semantics

### F. Tool Calling Semantics

OpenRouter tool calling is OpenAI-style:
- request includes `tools[]`
- optional `tool_choice`
- optional `parallel_tool_calls`
- assistant responses include `tool_calls[]` and typically `finish_reason:"tool_calls"`
- tool results are sent back as role `"tool"` messages keyed by `tool_call_id`
- tools must be included in every request in the workflow so router can validate schemas. citeturn7view5turn7view0turn3view0turn6view0turn24view2  

#### F.1 ToolCall id determinism

OpenRouter tool calls produce IDs like `call_abc123`, and tool results reference them via `tool_call_id`. citeturn7view0turn7view1turn24view2  

**Invariant**
- Use OpenRouter `tool_calls[].id` verbatim as `canonical ToolCall.id`. Never regenerate IDs. citeturn0view1turn7view1  

#### F.2 Tool call argument parsing rules

OpenRouter tool call function arguments are provided as a JSON string in `tool_calls[].function.arguments`. citeturn7view0turn8view0  

**Rules**
- Decode:
  - Parse the string as JSON.
  - On parse failure: store as JSON string in canonical `arguments_json` and warn; do not crash or guess. citeturn0view1turn5view1  
- Encode:
  - Serialize canonical `arguments_json` to a JSON string **deterministically**.
  - Recommended deterministic rule: recursively sort all object keys before serialization, so that semantically equal arguments map identically regardless of map insertion order. citeturn5view1turn0view1  

#### F.3 parallel_tool_calls behavior

OpenRouter parameter:
- `parallel_tool_calls` (boolean, default true; controls whether model may request multiple tool calls simultaneously). citeturn6view0turn7view4turn26view2  

Canonical has **no** field for this. citeturn0view1turn5view1  

**Adapter-private approach**
- Default: omit and let OpenRouter default apply, or set it explicitly using adapter config. citeturn6view0turn26view2  
- Canonical reflection:
  - Do not create canonical fields or metadata keys for it (leakage).
  - Canonical representation of multiple tool calls already exists via multiple `ContentPart::ToolCall`. citeturn0view1turn25view2  

#### F.4 Multiple tool calls and ordering

If OpenRouter returns multiple tool calls in a single assistant message (possible when parallel tool calls are enabled), preserve the array order and emit canonical content parts in that order. citeturn6view0turn25view2  

When both text and tool_calls are present in an assistant message, OpenRouter does not define an ordering between them (they are separate fields). The adapter must impose one:
- Emit `Text` first (if present), then `ToolCall` parts in tool_calls order. This is deterministic and stable. citeturn25view1turn0view1turn5view1  

#### F.5 Tool result formatting constraints

OpenRouter requires tool results as messages:
- `{ role:"tool", tool_call_id:<id>, content:<string> }` in the message list. citeturn7view1turn24view2  

And OpenRouter explicitly notes:
- The `tools` parameter must be included in every request (steps 1 and 3) so router can validate the tool schema on each call. citeturn3view0turn8view0turn7view5  

**Translator-side enforcement**
- If the outgoing message history contains any Tool-role message, require `req.tools` non-empty; otherwise return protocol error “tool results provided but tools schema missing.” citeturn3view0turn0view1  

#### F.6 Error signaling for tool execution

OpenRouter treats tool messages as content; there is no separate “tool execution error” channel in the protocol. citeturn7view5turn24view2  

Canonical policy:
- Tool execution errors should be represented by the harness as a tool result message content (e.g., JSON object or text) and fed back in the tool result content string. Translator just passes it through. citeturn0view1turn7view1  

## Canonical gaps, strict invariants, edge-case matrix

### H. Canonical gaps or required changes

This section is strictly about gaps **relative to OpenRouter’s documented surface** and your “no streaming / no leakage” constraints. citeturn5view1turn29view0turn26view2  

#### H.1 Gaps that can be handled via adapter-private config (no canonical changes required)

OpenRouter supports request knobs that canonical does not model:
- Routing knobs: `models[]` fallbacks, `provider` preferences, OpenRouter plugins. citeturn29view0turn3view4turn11view4  
- Additional sampling controls: `seed`, `frequency_penalty`, `presence_penalty`, etc. citeturn24view1turn26view2  
- Tool parallelism: `parallel_tool_calls`. citeturn6view0turn26view2  
- Observability fields: `user`, `session_id`, `trace` exist on the endpoint schema but are absent in canonical. citeturn4view2turn9view0turn26view2  

These can remain **adapter-private** without violating canonical provider-agnosticism.

#### H.2 Gaps that block “strict structured output correctness” under the current translator contract

OpenRouter structured outputs:
- can fail when unsupported or when schema invalid. citeturn12view4  
- are configured in the request via `response_format`. citeturn12view0turn3view8turn29view0  

But:
- `decode_response(&payload)` receives no `ProviderRequest`, so it cannot know whether structured output was requested or which schema to validate against. citeturn0view0turn0view1  

**Minimal, provider-agnostic change options**
- Option 1 (contract): extend translator contract decode signature to `decode_response(req, payload)` so schema and requested mode are available. This is provider-agnostic because every provider has “request intent” that is relevant to decoding. citeturn0view0turn5view3turn12view4  
- Option 2 (canonical response): add a canonical boolean like `structured_output_requested` into `ProviderResponse` so downstream can interpret parsing results. This is weaker than option 1 and still doesn’t enable schema validation. citeturn0view1turn12view4  

If you do not want canonical changes, you must accept a weaker guarantee: “structured_output is best-effort parse of assistant content if it looks like JSON.”

### I. Strict invariants

These invariants are designed to prevent router/provider transport leakage and ensure deterministic translation, matching Stage 18/19 constraints. citeturn5view1turn5view2turn5view3  

**Boundary invariants**
- Translator must never accept or produce canonical fields that contain OpenRouter-only objects (`provider`, `models`, `plugins`, `transforms`, `route`). Those are adapter-private only. citeturn29view0turn24view1turn5view2  
- Translator must not emit OpenRouter-specific field names into canonical warnings/messages (e.g., do not instruct users to set `provider.order` in canonical metadata). citeturn5view2turn0view1  
- Any “extra” provider payload data may only appear in `raw_provider_response` under an explicit debug opt-in policy; default must be `None`. citeturn0view1turn5view3turn15view5  

**Determinism invariants**
- Same canonical request encodes to byte-stable JSON:
  - deterministic ordering of selected map fields (`metadata` keys, serialized function arguments keys). citeturn5view1turn0view1  
- Same provider payload decodes identically:
  - always select `choices[0]` and ignore others with a warning if present. citeturn25view4turn5view1  

**Finish reason invariants**
- Always map OpenRouter finish reasons deterministically using the fixed mapping table; unknown/null becomes `FinishReason::Other`. citeturn25view0turn0view1  
- Do not surface `native_finish_reason` into canonical. citeturn25view0turn25view2  

**Tool invariants**
- If tool messages are present in `messages`, then `tools` must be present as well (schema validation requirement). citeturn3view0turn7view5turn0view1  
- `ToolChoice::Specific{name}` must match a tool in `tools[]` or fail encoding. citeturn6view0turn0view1  
- Tool call IDs must round-trip unchanged. citeturn7view0turn0view1  

**Error invariants**
- HTTP errors with OpenRouter error bodies map to `ProviderError` (adapter responsibility), and “HTTP 200 with embedded error” must still be treated as a failure. citeturn15view1turn15view4turn3view6  
- Error metadata fields containing provider names/raw provider errors must not be surfaced outside debug raw payload. citeturn15view2turn25view2turn21view1  

### J. Edge case matrix

This table enumerates the required edge cases, gives the expected OpenRouter wire shape at a high level, and the deterministic canonical outcome.

| Case | OpenRouter wire shape (high-level) | Canonical result | FinishReason | Validation outcome |
|---|---|---|---|---|
| Text-only response | `choices[0].message.content = "..."`, no `tool_calls` citeturn23search2turn25view1 | `output.content=[Text]`, `structured_output=None` | `Stop` (if `finish_reason="stop"`) citeturn25view0turn0view1 | OK |
| Tool-only response | `message.content=null`, `message.tool_calls=[...]`, `finish_reason="tool_calls"` citeturn7view0turn25view1 | `output.content=[ToolCall,...]` | `ToolCalls` citeturn25view0turn0view1 | OK |
| Text + tool_calls | `message.content="..."` and `tool_calls=[...]` citeturn25view1 | `output.content=[Text, ToolCall...]` deterministic ordering | `ToolCalls` (or other per payload) citeturn25view0 | OK (warn if finish_reason contradicts presence/absence of tool_calls) |
| Multiple tool_calls | `tool_calls=[tc1, tc2, ...]` citeturn6view0turn25view2 | Multiple `ContentPart::ToolCall` preserving order | `ToolCalls` | OK |
| Truncated output | `finish_reason="length"` citeturn25view0 | Normal text/toolcall mapping as present | `Length` citeturn0view1 | OK |
| Stop sequence triggered | `finish_reason="stop"` with `stop` used in request citeturn25view0turn9view0 | Normal mapping | `Stop` | OK |
| Content filtered | `finish_reason="content_filter"` (and content may be empty/refusal-like) citeturn25view0turn15view2 | Preserve whatever content exists; do not inject provider metadata | `ContentFilter` citeturn0view1 | OK |
| Provider/model fallback occurred | request used `models[]`; response `model` differs citeturn3view4turn25view2 | `ProviderResponse.model = response.model` only | per payload | OK; must not expose fallback list |
| Error returned (HTTP) | HTTP status != 200, body `{"error":{"code":...,"message":...}}` citeturn15view1 | Return `ProviderError` (adapter maps to `ProviderError::Status`/`CredentialsRejected`) citeturn21view1turn15view1 | N/A | Fail |
| Error embedded with HTTP 200 | `choices[0].finish_reason="error"` and/or `choice.error` present citeturn15view4turn25view1turn3view6 | Return `ProviderError::Protocol` (translator) | N/A | Fail |
| Empty output | `message.content=null`, no tool_calls; may occur on warmup citeturn15view2turn25view1 | `output.content=[]`, `warnings+=empty_output` | `Other` or mapped finish_reason | OK but warn |
| Usage missing/partial | `usage` absent or null (schema allows) citeturn9view3turn25view3 | usage fields None + warning | mapped finish_reason | OK but warn |
| Structured outputs requested but unsupported | request `response_format.type="json_schema"`; OpenRouter returns error “lack of support” citeturn12view4turn29view0 | Return `ProviderError` (Status/Protocol) | N/A | Fail |
| Structured outputs requested but invalid schema | request invalid schema; OpenRouter returns error citeturn12view4 | Return `ProviderError` | N/A | Fail |

### Sources used

```text
OpenRouter official docs (required set):
- https://openrouter.ai/docs/api/reference/overview
- https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request
- https://openrouter.ai/docs/api/reference/parameters
- https://openrouter.ai/docs/guides/features/tool-calling
- https://openrouter.ai/docs/guides/features/structured-outputs
- https://openrouter.ai/docs/guides/routing/provider-selection
- https://openrouter.ai/docs/guides/routing/model-fallbacks
- https://openrouter.ai/docs/api/reference/errors-and-debugging
- https://openrouter.ai/docs/api/reference/authentication
- https://openrouter.ai/docs/app-attribution
- https://openrouter.ai/openapi.json
- https://openrouter.ai/openapi.yaml (endpoint exists but was not fetchable in this session)
```

The OpenRouter adapter is complete when all canonical semantics are preserved without router/provider transport leakage and the translator contract is fully satisfied.
