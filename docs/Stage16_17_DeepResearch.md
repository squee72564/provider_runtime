# Anthropic Messages API Surface Mapping for provider-runtime Stage 16/17

I’m approaching this as an LLM API protocol + Rust runtime boundary specialist, focusing on a deterministic, contract-faithful Anthropic Messages (non‑streaming) translator design that keeps your canonical layer provider‑agnostic.

## Translator contract alignment

**A. Translator Contract Alignment**

Your crate-private translator contract is intentionally tiny:

- `trait ProviderTranslator` with:
  - `type RequestPayload`
  - `type ResponsePayload`
  - `fn encode_request(&self, req: &ProviderRequest) -> Result<Self::RequestPayload, ProviderError>`
  - `fn decode_response(&self, payload: &Self::ResponsePayload) -> Result<ProviderResponse, ProviderError>` citeturn2view0

This means the Anthropic surface has to be designed as:

- **RequestPayload**: a pure *wire-ready* representation of a non-streaming `POST /v1/messages` request body **plus** any deterministic warnings you want to propagate to the runtime (the OpenAI translator already follows this “payload + warnings” pattern). citeturn11view3turn2view2  
- **ResponsePayload**: the decoded JSON object from a completed non-streaming Messages call, plus any “out-of-band” context needed for deterministic decoding (notably: the originally requested `ResponseFormat`, if you want structured-output parsing to be deterministic without re-inspecting caller intent elsewhere). citeturn19view0turn2view0

**Responsibilities per method**

`encode_request(req)`

- **Responsibility**: Convert canonical `ProviderRequest` into an Anthropic Messages request body (`model`, `max_tokens`, `messages`, optional `system`, optional `tools`, optional `tool_choice`, optional `output_config`, optional `stop_sequences`, optional `temperature/top_p`, optional `metadata`). citeturn17view1turn7view0turn6view2turn17view0  
- **Input invariants (enforced here, not leaked)**:
  - Provider hint, if present, must match `ProviderId::Anthropic`. citeturn19view0turn11view2
  - Canonical message invariants must be met in a way that survives Anthropic’s “consecutive same-role turns are combined” rule. citeturn17view1
  - Tool-use / tool-result ordering constraints must be enforced *inside* encoding so the adapter never emits an invalid wire shape. citeturn9view2
- **Output guarantees**:
  - The produced body is syntactically valid JSON for `POST /v1/messages` in non-streaming mode. citeturn17view1turn1view0
  - Deterministic warnings list (stable codes/messages) for any tolerated lossy behavior.
- **Error conditions**:
  - Canonical intent cannot be represented without semantic loss (e.g., mid-thread system messages). citeturn5view2turn19view0
  - Tool ordering constraints violated (e.g., tool results not immediately after tool use). citeturn9view2
- **Must not leak**:
  - No Anthropic enum strings (e.g., `"end_turn"`, `"tool_use"`) are exposed in canonical output. Mapping to canonical `FinishReason` happens in `decode_response`. citeturn7view3turn19view0  
  - No raw provider JSON fields are surfaced outside `raw_provider_response` (which remains `None` by default in v0). citeturn19view0turn10view2

`decode_response(payload)`

- **Responsibility**: Convert a successful non-streaming Messages response JSON into canonical `ProviderResponse`:
  - Parse `content[]` blocks deterministically (text/tool_use/thinking/etc.)
  - Map `stop_reason` to canonical `FinishReason`
  - Normalize `usage` into canonical `Usage`
  - Derive `structured_output` when requested and possible (strict rules below) citeturn17view3turn7view3turn21view2turn18view4turn19view0
- **Input invariants**:
  - Payload must be a JSON object matching Message response shape (at minimum: `role=assistant`, `content` array, `stop_reason`, `usage` object possibly present). citeturn1view0turn17view3turn7view3
- **Output guarantees**:
  - Canonical `ProviderResponse` always returned on success:
    - `provider = ProviderId::Anthropic`
    - `model` set from response `model` string
    - `finish_reason` always set deterministically
    - `warnings` stable and deterministic for same payload citeturn19view0turn2view2
- **Error conditions**:
  - Malformed payload shape (missing required fields, wrong types) become `ProviderError::Protocol` or `ProviderError::Serialization` (per your error taxonomy). citeturn11view2turn2view0
- **Must not leak**:
  - Do not pass through raw `stop_reason` strings or block type strings as canonical strings; only canonical enums and structured values. citeturn7view3turn17view3turn19view0

**Stage alignment**

- Stage 16 explicitly scopes the translator to canonical→Anthropic and Anthropic→canonical mapping via the shared translator boundary. citeturn2view2turn11view0  
- Stage 17 scopes the Anthropic adapter to orchestration only; all field mapping must be in the translator. citeturn2view3turn10view2

## Canonical to Anthropic Messages request mapping

**B. Canonical → Anthropic Messages Request Mapping**

Anthropic Messages is `POST /v1/messages` on `https://api.anthropic.com`. citeturn1view0turn17view1  
Non-streaming mode is the default for a normal `POST /v1/messages` without SSE. (You must *not* use streaming per your constraints.) citeturn17view1turn6view3

### Canonical field mapping table

| Canonical field | Anthropic field / behavior | Transformation rules | Validation / invalid states | Edge cases |
|---|---|---|---|---|
| `req.model.model_id` | `model` | Copy string verbatim. | Error if empty / whitespace-only. citeturn17view1turn19view0 | None. |
| `req.model.provider_hint` | *not sent* (validate only) | If `Some`, must equal `Anthropic`, else error. citeturn19view0turn11view2 | Provider hint mismatch. | None. |
| `req.max_output_tokens` | `max_tokens` (required by Anthropic) | If `Some(n)`, set `max_tokens=n`. If `None`, apply deterministic fallback described below (because Anthropic requires `max_tokens`). citeturn17view1turn19view0 | `max_tokens` must be ≥ 1. citeturn17view1 | See “default max_tokens fallback” below. |
| `req.stop: Vec<String>` | `stop_sequences` | Copy array.citeturn6view2turn19view0 | Reject empty strings; reject if any element not valid UTF-8 string (canonical is String, so only empty check matters). | If stop sequence hit, response includes `stop_reason="stop_sequence"` + `stop_sequence` value. citeturn6view2turn7view3 |
| `req.temperature` | `temperature` | Copy if present. Anthropic range is 0.0–1.0, default 1.0. citeturn3view8 | Error if outside [0,1]. | Even temperature 0 isn’t fully deterministic (doc note). citeturn3view8 |
| `req.top_p` | `top_p` | Copy if present. citeturn17view2 | Error if outside [0,1]. citeturn17view2 | Anthropic recommends not using both `temperature` and `top_p` simultaneously (warning-worthy). citeturn17view2turn3view8 |
| `req.metadata: BTreeMap<String,String>` | `metadata.user_id` only | Map **only** canonical key `"user_id"` to Anthropic `metadata.user_id`. Anthropic metadata schema is `{ user_id }`. citeturn17view0turn6view0turn19view0 | Error if `user_id` value > 256 chars (Anthropic maxLength 256). citeturn17view0 | All other metadata keys: cannot be represented; must be dropped with deterministic warning (no silent loss). |
| `req.response_format` | Prefer `output_config.format` (JSON outputs) when JsonObject/JsonSchema; otherwise none | Mapping detailed in “Structured output handling”. `output_config.format.type="json_schema"` with a JSON schema. citeturn17view0turn18view4turn19view0 | JSON outputs incompatible with message prefilling. citeturn18view1turn18view2 | If stop_reason is refusal or max_tokens, schema compliance may fail even with structured outputs. citeturn18view5turn7view3 |
| `req.tools: Vec<ToolDefinition>` | `tools[]` (client tools) | Each tool: `{ "name", "description"?, "input_schema": parameters_schema }`.citeturn7view0turn20view0turn19view0 | Tool name length 1–128; reject empty.citeturn20view0 Tool schema must be JSON object. citeturn19view0turn7view0 | “strict tools” exist, but canonical cannot express strict yet (gap). citeturn20view0turn18view5 |
| `req.tool_choice` | `tool_choice` | `None`→`{"type":"none"}`; `Auto`→`{"type":"auto"}`; `Required`→`{"type":"any"}`; `Specific{name}`→`{"type":"tool","name":name}`. citeturn7view0turn19view0 | `Specific` must reference an existing tool by name. Error if missing. | `disable_parallel_tool_use` has no canonical representation; defaulting rules below. citeturn7view0turn9view5 |
| `req.messages: Vec<Message>` | `system` + `messages[]` | System messages extracted to top-level `system`; user/assistant/tool mapped into `messages[]` with content blocks. Anthropic has no `"system"` role in messages. citeturn5view2turn17view1turn19view0 | Mid-thread system messages cannot be represented without semantic change → error. | Consecutive same-role turns will be combined by Anthropic → must pre-merge deterministically. citeturn17view1turn5view0 |

### Canonical message and content mapping

**Role mapping**

Anthropic input messages allow only `"user"` and `"assistant"` roles. citeturn5view4turn5view5  
Additionally, Anthropic explicitly states:

- There is no `"system"` role in input messages; use top-level `system`. citeturn5view2  
- Consecutive `user` or `assistant` turns “will be combined into a single turn.” citeturn17view1turn5view0

Canonical roles and safe mapping:

- `MessageRole::System` → top-level `system` parameter. citeturn5view2turn6view3  
- `MessageRole::User` → `{ role: "user", content: [...] }`. citeturn17view1turn19view0  
- `MessageRole::Assistant` → `{ role: "assistant", content: [...] }`. citeturn17view1turn19view0  
- `MessageRole::Tool` → `{ role: "user", content: [ tool_result blocks... ] }` (since Anthropic places tool results in user messages). citeturn9view2turn9view5turn19view0  

**System prompt mapping rules**

Anthropic `system` supports either a string or an array of text blocks. citeturn6view3turn3view5  
Canonical system messages are richer than Anthropic’s “single system channel”, so the safe invariant is:

- **Invariant**: All canonical `System` messages must appear as a contiguous prefix of `req.messages` (before the first non-System message). Otherwise, return `ProviderError::Protocol` with deterministic message (cannot preserve system priority mid-thread under this API shape). citeturn5view2turn19view0

Mapping:

- Collect prefix System messages.
- Convert each `ContentPart::Text` into a `{"type":"text","text":...}` block.
- If only one block, you may emit system as a single string; otherwise emit the array form (deterministically prefer array form to avoid ambiguity). citeturn6view3turn17view1turn19view0

**ContentPart → Claude content blocks**

Anthropic message `content` can be a string shorthand for a one-element `{"type":"text"}` array, or an array of typed content blocks. citeturn17view1turn5view4  
For deterministic translation, always emit the explicit array form.

Canonical `ContentPart` mapping:

- `ContentPart::Text { text }` → `{ "type": "text", "text": text }`. citeturn5view4turn17view3turn19view0  
- `ContentPart::ToolCall { tool_call }` → `{ "type": "tool_use", "id": tool_call.id, "name": tool_call.name, "input": tool_call.arguments_json }`. Tool use blocks have `id`, `name`, `input` object. citeturn7view0turn7view2turn17view3turn19view0  
  - **Validation**: `arguments_json` must be a JSON object (map). If not, error; Anthropic defines tool input as an object/map. citeturn7view2turn9view5  
- `ContentPart::ToolResult { tool_result }` → `{ "type": "tool_result", "tool_use_id": tool_result.tool_call_id, "content": ... , "is_error": ...? }`. Tool result content can be string or a list of content blocks; and `is_error` is optional. citeturn9view2turn20view0turn7view0turn19view0  
  - Canonical has no `is_error`; treat all tool results as non-error unless the caller encodes error info in text content.
  - Canonical tool result “content” only supports text/thinking/toolcall/toolresult; but Anthropic tool_result nested content supports text/image/document. If canonical includes any non-text parts inside a tool result, return `ProviderError::Protocol` (no silent transformation). citeturn9view2turn19view0turn2view2
- `ContentPart::Thinking { ... }` in **requests**: must not be passed through by default. Extended thinking blocks exist as content block types, but keeping cross-provider thinking in prompt history is explicitly sensitive in your v0 plan (“handoff rule” manages this). The safe, provider-agnostic invariant is “drop thinking on encode; warn deterministically.” citeturn10view0turn19view0turn17view3

### Consecutive same-role merging and the tool-result hazard

Anthropic will combine consecutive `user` turns; that is not optional. citeturn17view1turn5view0  
Anthropic also requires, for tool results:

- Tool result blocks must immediately follow their corresponding tool use blocks in message history (no messages in between).
- In the user message containing tool results, tool_result blocks must come **first** in the content array; any text must come **after** all tool results. citeturn9view2

This creates a concrete adapter hazard:

- Canonical often represents tool execution as `Assistant(tool_use)` then `Tool(tool_result)` then `User(next question)`.
- Mapping `Tool`→`user` creates consecutive `user` turns (`tool_result` then `next question`), and Anthropic would combine them.
- If the combined content puts user text before tool_result, Anthropic returns a 400. citeturn9view2turn17view1

**Therefore the translator must pre-merge canonical messages into an Anthropic-safe sequence**:

- After mapping roles, merge consecutive `"user"` messages into a single user message.
- When merging, preserve the tool_result-first rule by concatenating:
  1) all tool_result blocks (in the original order they appeared in the consecutive user group), then  
  2) all non-tool_result blocks (typically text). citeturn9view2turn17view1

This is a deterministic normalization that protects canonical semantics *and* respects the Anthropic ordering constraint.

### Tool definition mapping

Anthropic client tool definitions use:

- `name` (required)
- `description` (optional but recommended)
- `input_schema` (JSON Schema describing tool input) citeturn7view0turn20view0

Canonical:
- `ToolDefinition { name, description, parameters_schema }` citeturn19view0

Mapping:

- `name` → `name`
- `description` → `description` if present
- `parameters_schema` → `input_schema`

Validation:

- tool name must be non-empty; Anthropic also constrains length 1–128. citeturn20view0  
- `parameters_schema` must be a JSON object (not an array/string). citeturn7view0turn19view0

### ToolChoice mapping and disable_parallel_tool_use

Anthropic `tool_choice` accepts:

- `{"type":"auto"}` optionally with `disable_parallel_tool_use`
- `{"type":"any"}` optionally with `disable_parallel_tool_use`
- `{"type":"tool","name":"..."}` optionally with `disable_parallel_tool_use`
- `{"type":"none"}` citeturn7view0turn3view0

And semantics of `disable_parallel_tool_use`:

- If `true`:
  - for `auto`: model uses at most one tool use
  - for `any` / `tool`: model outputs exactly one tool use citeturn7view0turn9view5

Canonical has no place to express parallel tool policy. citeturn19view0  
So the translator must choose a deterministic default **without leaking provider specifics**:

- **Default**: do not set `disable_parallel_tool_use` (let Anthropic default `false`). citeturn7view0turn9view5  
- **Exception** (recommended for semantic alignment with canonical `Specific`): set `disable_parallel_tool_use=true` on `Specific { name }` to reduce “multiple tool calls when a single forced tool is intended.” This mirrors the intent of forcing a specific tool in a deterministic way across providers. (This is a design choice; the doc only defines behavior, not your desired default.) citeturn7view0turn10view1turn19view0

### Default max_tokens fallback

Anthropic requires `max_tokens`. citeturn17view1turn5view3  
Canonical `max_output_tokens` is optional by design. citeturn19view0turn11view1

To avoid rejecting valid canonical requests, define:

- If `req.max_output_tokens` is `None`, emit a deterministic fallback `max_tokens = 1024`.

This matches the canonical “optional controls present/absent” test category in Stage 16, while meeting Anthropic’s required field constraint. citeturn2view2turn17view1turn1view0  
Because this fallback is not provider-specified (it’s a translator policy), the safe behavior is to also emit a deterministic warning (see warning scheme in the Invariants section).

## Anthropic response to canonical mapping

**C. Anthropic Response → Canonical Mapping**

A successful non-streaming Messages response returns a `Message` object with:

- `role` always `"assistant"`
- `content`: array of content blocks
- `model`
- `stop_reason` (non-null in non-streaming mode)
- optional `stop_sequence`
- `usage` object with token counts citeturn1view0turn7view3turn17view3turn21view2

### Deterministic content[] parsing

Anthropic `content` blocks that matter for your canonical types include:

- `"text"` blocks with a `text` field citeturn17view3turn5view4  
- `"tool_use"` blocks with `{id, name, input}` citeturn17view3turn9view5  
- `"thinking"` and `"redacted_thinking"` blocks (if enabled) citeturn17view3turn7view2  
- Server tool blocks like `"server_tool_use"` and `"web_search_tool_result"` exist in the response schema, but your canonical layer has no first-class representation yet. citeturn17view3turn21view5

**Ordering rule**: preserve the exact block order in canonical `AssistantOutput.content`, so the caller can reliably reconstruct the mixed “text then tool_use” patterns. Anthropic explicitly warns that tool call explanatory text is natural language and may vary; your code must treat it like ordinary assistant text. citeturn9view5

Canonical conversion per block:

- `type="text"` → `ContentPart::Text { text }` citeturn17view3turn19view0  
- `type="tool_use"` → `ContentPart::ToolCall { ToolCall { id, name, arguments_json: input } }` citeturn17view3turn19view0  
  - **Validation**: if `input` is not an object/map, treat as protocol error (tool input is specified as map/object). citeturn17view3turn9view5  
- `type="thinking"` → `ContentPart::Thinking { text: thinking, provider: Some("anthropic") }` (provider tag is allowed by canonical schema). citeturn17view3turn19view0  
- `type="redacted_thinking"` → `ContentPart::Thinking { text: "<redacted>", provider: Some("anthropic") }` **plus warning** (since canonical currently has no redaction flag). citeturn17view3turn19view0  
- Any other block types (images, documents, server tools, citations blocks) → convert to `ContentPart::Text` using a deterministic JSON stringification of the block **plus warning** (not silent), or return protocol error if you prefer strictness. The response schema includes multiple such block families. citeturn17view3turn21view5turn16search0

### Stop reason, stop sequence, and multi-turn continuation behavior

Anthropic lists these stop reasons:

- `end_turn`
- `max_tokens`
- `stop_sequence`
- `tool_use`
- `pause_turn`
- `refusal` citeturn7view3turn3view2

And clarifies `stop_sequence` is populated when `stop_reason="stop_sequence"`. citeturn7view3turn6view2

Tool-use loop semantics (client tools):

- When using client tools, the response has `stop_reason="tool_use"` and one or more `tool_use` blocks.
- The client should execute the tool(s) and continue by sending a **new user message** containing the corresponding `tool_result` blocks. citeturn9view5turn9view2turn7view0

Pause semantics:

- `pause_turn` indicates the API paused a long-running turn; the docs state you may provide the response back “as-is” to let the model continue. citeturn7view3turn9view2

### Usage parsing

Anthropic `usage` includes:

- `input_tokens`
- `output_tokens`
- `cache_creation_input_tokens`
- `cache_read_input_tokens`
- `cache_creation` breakdown by TTL
- plus other fields like `inference_geo`, `service_tier`, `server_tool_use` citeturn7view3turn21view0turn21view2

Important token semantics:

- Token counts won’t match visible text 1:1 due to internal transformations.
- `output_tokens` can be non-zero even for an “empty string response”.
- Total input tokens for billing is `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. citeturn21view2turn21view0

Canonical `Usage` mapping (deterministic):

- `Usage.input_tokens` = `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` (the billed total input). citeturn21view0turn19view0  
- `Usage.cached_input_tokens` = `cache_read_input_tokens` (cached reads). citeturn21view1turn19view0  
- `Usage.output_tokens` = `output_tokens`. citeturn21view2turn19view0  
- `Usage.reasoning_tokens` = `None` (Anthropic does not provide a separate reasoning token counter in this response schema; thinking is included in output tokens). citeturn17view3turn21view2turn19view0  
- `Usage.total_tokens` = `input_tokens_billed + output_tokens` (set explicitly so consumers don’t rely on derived totals). citeturn19view0turn21view0

### Error handling surfaces

**HTTP + error body shapes** (for the adapter boundary, but must be designed here)

Anthropic’s error docs specify:

- Standard HTTP status mapping to error “types”:
  - 400 invalid_request_error
  - 401 authentication_error
  - 403 permission_error
  - 404 not_found_error
  - 413 request_too_large
  - 429 rate_limit_error
  - 500 api_error
  - 529 overloaded_error citeturn1view1
- Error responses are JSON:
  - Top-level `"type": "error"`
  - `"error": { "type": ..., "message": ... }`
  - `"request_id": ...`
- Error `type` values may expand over time. citeturn1view1

Your canonical error taxonomy includes `ProviderError::{CredentialsRejected, Transport, Status, Protocol, Serialization}`. citeturn11view2turn2view0

**Mapping policy (design)**:

- 401 → `ProviderError::CredentialsRejected { provider=Anthropic, request_id?, message }` citeturn1view1turn11view2  
- Any other non-2xx HTTP response with JSON error body:
  - Parse `error.type`, `error.message`, and `request_id`
  - Return `ProviderError::Status { provider=Anthropic, status_code, request_id, model: Option, message: formatted }` citeturn1view1turn11view2  
- If body is not parseable JSON, fall back to a plain-text message and still return `ProviderError::Status`. This matches the “no silent failure” rule.

Because the translator contract does not include an explicit “decode_error” method, the practical design is: the Anthropic translator module defines a deterministic `parse_anthropic_error_envelope(body: &str)` helper for the adapter to call (mirroring the OpenAI translator pattern), but the adapter remains responsible for HTTP orchestration per Stage 17. citeturn2view3turn1view1turn2view0

## Structured output handling

**D. Structured Output Handling**

Canonical has:

- `ResponseFormat::Text`
- `ResponseFormat::JsonObject`
- `ResponseFormat::JsonSchema { name, schema }` citeturn19view0turn11view1

Anthropic does **not** have an OpenAI-style “JSON mode”. Instead, official, supported ways are:

- **JSON outputs** via `output_config.format` with `type: "json_schema"` (structured outputs) citeturn18view4turn17view0  
- **Strict tool use** via setting `"strict": true` on tool definitions to validate tool names and tool inputs citeturn18view5turn20view0  
- Prompt-only approaches (best-effort, not guaranteed); Anthropic explicitly recommends using Structured Outputs when you need guaranteed schema conformance. citeturn12search6turn18view4

### Strategy: JSON outputs via output_config.format

Anthropic Structured Outputs documentation says:

- Put schema under `output_config.format` with `{"type":"json_schema","schema":{...}}`
- The response is valid JSON matching the schema in `response.content[0].text`. citeturn18view4turn17view0

**Mapping rules**

- Canonical `Text`:
  - Do **not** set `output_config.format`. Keep response as normal text. citeturn17view0turn19view0
- Canonical `JsonSchema { name, schema }`:
  - Set `output_config.format = { type:"json_schema", schema: <canonical schema> }`. `name` has no Anthropic analog; it is used only for canonical identity/diagnostics, not transmitted. citeturn17view0turn18view4turn19view0
- Canonical `JsonObject`:
  - Prefer JSON outputs using a generic object schema (example):
    - `{ "type":"object", "additionalProperties": true }`
  - Rationale: keeps behavior “guaranteed valid JSON object” without asking the caller to embed fragile prompt constraints. citeturn18view4turn12search6turn19view0

**Incompatibility guardrails**

Structured outputs’ “Feature compatibility” section:

- JSON outputs are incompatible with **Message Prefilling** (ending input messages with an assistant turn to force continuation). citeturn18view1turn18view2turn17view1

Therefore:

- If `req.response_format` is `JsonObject` or `JsonSchema`, and the (post-normalization) last message role is `assistant`, return `ProviderError::Protocol` (cannot preserve both “prefill continuation” semantics and “JSON outputs” guarantee). citeturn18view2turn17view1turn19view0

**Invalid structured outputs scenarios**

Structured outputs doc warns output may fail schema in these cases:

- Refusals (`stop_reason="refusal"`) still return **HTTP 200**, output may not match schema. citeturn18view5turn7view3  
- Token limit reached (`stop_reason="max_tokens"`) may cut off JSON. citeturn18view5turn7view3

So decoding behavior must be:

- Attempt JSON parse only when caller requested structured output **and** first content block is `text`.
- If parse fails:
  - `structured_output = None`
  - emit deterministic warning (e.g., `structured_output_parse_failed`)
  - keep raw text content in `output.content` so callers can debug or recover. citeturn18view4turn18view5turn19view0

### Strategy: prompt-constrained JSON-only output

This is a fallback strategy when JSON outputs cannot be used (e.g., model incompatibility, or you explicitly choose not to rely on `output_config.format`).

Design constraints:

- The translator should not inject provider-specific prompt text into canonical messages unless you treat that as part of canonical `ResponseFormat` semantics (otherwise it’s hidden behavior).
- Given your “no silent lossy conversion” principle, prompt-only mode should be explicitly “best effort” and must emit warnings when JSON parsing is used heuristically. citeturn2view2turn12search6turn19view0

Deterministic extraction rule (design):

- If `ResponseFormat::JsonObject`, parse the **first complete JSON object** found in concatenated assistant text blocks, preferring:
  1) `response.content[0].text` if it begins with `{`
  2) otherwise the first `{...}` region that parses fully
- On success, set `structured_output` to parsed object; on failure, keep `structured_output=None` + warning. citeturn19view0turn18view4

### Strategy: schema-guided output via tools (strict tool use)

Anthropic Structured Outputs describes strict tool use:

- Add `"strict": true` to tool definitions; then tool `name` is always valid and tool `input` strictly follows `input_schema`.
- Tool-use blocks expose validated inputs in `response.content[x].input`. citeturn18view5turn20view0

Anthropic also notes tools can be used “whenever you want the model to produce a particular JSON structure of output.” citeturn20view0turn9view5

**Canonical alignment reality check**

- Canonical already supports tool calls (`ContentPart::ToolCall`) and tool results (`ContentPart::ToolResult`). citeturn19view0  
- However, canonical does **not** currently let a caller specify `"strict": true` on tool definitions, so you cannot fully express this strategy without a canonical extension (see gap analysis). citeturn19view0turn18view5

**How to treat tool-based schema output in canonical**

If a caller wants the *tool call input itself* to be the structured output:

- Decode normally into `ContentPart::ToolCall`.
- Additionally, if the canonical request’s `response_format` is `JsonSchema`/`JsonObject`, and the assistant message contains exactly one tool call and no text, you *may* set `structured_output = tool_call.arguments_json` with a deterministic warning like “structured_output_from_tool_call”.  
This does not leak provider details; it’s a canonical-level policy about interpreting tool call arguments as structured output.

## Finish reason mapping, tool semantics, and usage mapping

**E. Finish Reason Mapping Matrix**

Anthropic `stop_reason` values and canonical `FinishReason` mapping (deterministic):

| Anthropic `stop_reason` | Meaning (Anthropic) | Canonical `FinishReason` | Notes |
|---|---|---|---|
| `end_turn` | natural stopping point | `Stop` | citeturn7view3turn19view0 |
| `stop_sequence` | matched one of your `stop_sequences` | `Stop` | `stop_sequence` contains matched sequence. citeturn6view2turn7view3 |
| `max_tokens` | exceeded requested `max_tokens` or model max | `Length` | Also affects structured outputs validity. citeturn7view3turn18view5turn19view0 |
| `tool_use` | model invoked one or more tools | `ToolCalls` | Tool calls appear as `tool_use` blocks. citeturn7view3turn9view5turn19view0 |
| `pause_turn` | paused long-running turn; you may send response back to continue | `Other` | Canonical has no dedicated “paused” reason. citeturn7view3turn9view2turn19view0 |
| `refusal` | safety refusal; in structured outputs docs it returns 200 but may not match schema | `ContentFilter` | Deterministic warning recommended. citeturn7view3turn18view5turn19view0 |
| *(unknown future value)* | docs allow expansion over time | `Other` | Emit warning `unknown_stop_reason`. citeturn1view1turn7view3turn19view0 |

**F. Tool Calling Semantics**

Tool call identity:

- Use the tool_use block’s `id` as canonical `ToolCall.id` verbatim (deterministic, round-trippable). Tool_use blocks include `id` and tool_result blocks must reference it via `tool_use_id`. citeturn7view0turn9view5turn9view2turn19view0

Tool call arguments:

- Anthropic specifies `input` as an object (map). citeturn17view3turn7view2  
- Canonical `ToolCall.arguments_json` may be any JSON value. For Anthropic:
  - Require it to be an object at encode time.
  - If it is not, error with deterministic message (no silent coercion). citeturn7view2turn19view0turn2view2

Parallel tool use:

- Anthropic may call multiple tools by default; `disable_parallel_tool_use` can constrain this:
  - `auto` + disable_parallel=true ⇒ at most one tool
  - `any/tool` + disable_parallel=true ⇒ exactly one tool citeturn7view0turn9view5  
- Canonical lacks a parallelism control; translator must pick defaults (documented earlier). citeturn19view0

Text + tool_use coexistence:

- Anthropic notes the assistant may include natural language text before tool_use blocks; code must treat that like normal assistant text and not rely on specific phrasing. citeturn9view5  
- Therefore, decoding preserves mixed ordering: `Text` parts and `ToolCall` parts stay in the same sequence as `content[]`.

Tool result constraints (encoding):

- Tool result blocks must immediately follow their corresponding tool use blocks; no intervening messages.
- In the user message containing tool results, tool_result blocks must be **first** in the `content` array; any text must come after. citeturn9view2

Failure modes:

- tool_result missing after tool_use in conversational history you send → request invalid, likely 400; translator must error before sending. citeturn9view2turn1view1  
- tool_result references unknown tool_use_id → error (translator-side), to prevent sending invalid history. citeturn9view2turn19view0  
- tool_result `is_error` exists in Anthropic schema, but canonical has no flag; encode cannot represent it. citeturn9view2turn19view0

**G. Usage Mapping**

Anthropic usage rules:

- Total input tokens billed = `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`. citeturn21view0  
- output_tokens can be non-zero even for an empty string response. citeturn21view2

Canonical mapping (recommended, deterministic):

- `Usage.input_tokens = billed_input_total`
- `Usage.cached_input_tokens = cache_read_input_tokens`
- `Usage.output_tokens = output_tokens`
- `Usage.total_tokens = billed_input_total + output_tokens`
- Leave `Usage.reasoning_tokens = None` (not provided separately). citeturn21view0turn21view2turn19view0

## Validation rules, canonical gaps, invariants, and edge cases

**H. Canonical Gaps or Required Changes**

Several Anthropic capabilities cannot be expressed through your current canonical types without either losing fidelity or introducing hidden translator behavior:

- **Strict tool use (`strict: true`) is not representable**:
  - Anthropic supports `strict: true` on tools to guarantee schema validation on tool names and inputs. citeturn18view5turn20view0  
  - Canonical `ToolDefinition` has no strict flag. citeturn19view0  
  - **Minimal provider-agnostic change**: add `ToolDefinition.strict: Option<bool>` (or a nested `ToolOptions` struct) with semantics “provider should validate tool-call args against schema when supported.” This is not Anthropic-specific; it generalizes to “strict tool schema execution”.  
- **Cache usage breakdown cannot be preserved**:
  - Anthropic reports `cache_creation_input_tokens` and per‑TTL breakdown in `cache_creation`. citeturn21view0turn21view4  
  - Canonical only has `cached_input_tokens` (read tokens) and no “cache creation” fields. citeturn19view0  
  - **Minimal change**: extend `Usage` with `cache_creation_input_tokens: Option<u64>` and optionally a generic breakdown map (still provider-agnostic: caching exists as a concept beyond Anthropic).
- **Thinking request configuration is absent**:
  - Anthropic supports a `thinking` request parameter to enable thinking blocks. citeturn7view1turn17view2  
  - Canonical has `ContentPart::Thinking` and provider capabilities support thinking, but `ProviderRequest` has no way to request thinking. citeturn19view0turn10view2  
  - If you intend to support “thinking on/off” as user intent, canonical needs a request-level field (provider-agnostic) such as `thinking: Option<ThinkingRequest>`.
- **Metadata is underspecified and mismatched**:
  - Anthropic metadata only supports `user_id`. citeturn17view0turn6view0  
  - Canonical metadata is arbitrary key/value map. citeturn19view0  
  - No canonical change is strictly required (you can drop unsupported keys with warnings), but you may want to standardize reserved keys like `"user_id"` across providers.

**I. Strict Invariants**

These invariants prevent provider leakage and enforce deterministic, safe translation:

- Canonical output must not contain Anthropic transport strings:
  - No `"stop_reason"` values (`"end_turn"`, `"tool_use"`, etc.) leave the translator; they map to canonical `FinishReason`. citeturn7view3turn19view0  
  - No Anthropic block type strings (`"tool_use"`, `"thinking"`, etc.) are stored as strings; they map to `ContentPart` variants. citeturn17view3turn19view0
- The translator is the only place where:
  - system prompt is lifted to top-level `system` (since there is no `"system"` role). citeturn5view2turn19view0  
  - consecutive same-role messages are pre-merged to avoid unintended Anthropic combining from changing tool_result ordering rules. citeturn17view1turn9view2
- Tool ordering invariants enforced at encode:
  - No message may appear between an assistant tool_use message and the user tool_result message.
  - In user message containing tool results, tool_result blocks must come first. citeturn9view2
- ToolChoice invariants:
  - `ToolChoice::Specific` must specify a non-empty name and must reference a known tool definition.
  - If tools are empty and ToolChoice is `Required` or `Specific`, return a deterministic error (cannot satisfy tool choice semantics). citeturn7view0turn19view0
- Structured outputs invariants:
  - If using `output_config.format`, do not allow message prefilling (assistant final input message) in the same request. citeturn18view2turn17view1
  - If `stop_reason` is `refusal` or `max_tokens`, do not assume schema validity; parsing must be best-effort with warnings. citeturn18view5turn7view3

**Deterministic translation algorithms (pseudocode)**

Encode (canonical → Anthropic request body):

```text
function encode_request(req):
  assert provider_hint is None or Anthropic
  assert model_id non-empty

  warnings = []

  # 1) Extract system prefix
  (system_msgs, rest_msgs) = split_prefix(req.messages where role==System)
  if any System message exists in rest_msgs:
      error("system messages must be prefix for Anthropic")

  system_blocks = flatten_text_parts(system_msgs)
  if system_blocks empty:
      system_field = null
  else:
      system_field = system_blocks_as_array  # deterministic

  # 2) Map messages to Anthropic roles and blocks
  mapped = []
  for msg in rest_msgs:
     if msg.role==User: role="user"
     if msg.role==Assistant: role="assistant"
     if msg.role==Tool: role="user"
     blocks = []
     for part in msg.content:
         match part:
           Text(t) -> blocks.push(text_block(t))
           ToolCall(c) -> blocks.push(tool_use_block(c))  # input must be object
           ToolResult(r) -> blocks.push(tool_result_block(r))  # only allowed inside Tool messages
           Thinking(_) -> drop + warnings.push("dropped_thinking_on_encode")
     validate blocks not empty (unless explicitly allowed)
     mapped.push({role, blocks, original_role=msg.role})

  # 3) Pre-merge consecutive same-role turns (because Anthropic will combine)
  merged = []
  for entry in mapped:
     if merged.last.role != entry.role:
         merged.push(entry)
     else:
         merged.last.blocks = merge_user_blocks_safely(merged.last.blocks, entry.blocks)

  # 4) Enforce tool ordering constraints
  enforce_tool_result_ordering(merged)

  # 5) Map tools + tool_choice
  tools = map_tools(req.tools)  # name/description/input_schema
  tool_choice = map_tool_choice(req.tool_choice, tools)

  # 6) Map stop, temperature, top_p, metadata.user_id
  stop_sequences = req.stop
  temp = req.temperature
  top_p = req.top_p
  if temp and top_p: warnings.push("both_temperature_and_top_p_set")

  anthropic_metadata = {}
  if "user_id" in req.metadata: anthropic_metadata.user_id = req.metadata["user_id"]
  if other metadata keys: warnings.push("dropped_unsupported_metadata_keys")

  # 7) Map response_format to output_config (see structured output rules)
  output_config = map_output_config(req.response_format, merged)

  # 8) max_tokens
  if req.max_output_tokens is None:
      max_tokens = 1024
      warnings.push("default_max_tokens_applied")
  else:
      max_tokens = req.max_output_tokens

  body = {
     "model": req.model.model_id,
     "max_tokens": max_tokens,
     "messages": merged_as_messageparam_array,
     optional "system": system_field,
     optional "tools": tools,
     optional "tool_choice": tool_choice,
     optional "output_config": output_config,
     optional "stop_sequences": stop_sequences,
     optional "temperature": temp,
     optional "top_p": top_p,
     optional "metadata": anthropic_metadata (only if user_id set)
  }

  return { body, warnings }
```

Decode (Anthropic response → canonical):

```text
function decode_response(payload):
  ensure payload is object with role=="assistant"
  model = payload.model or ""
  stop_reason = payload.stop_reason (must exist in non-streaming)
  finish_reason = map_stop_reason(stop_reason)

  content_parts = []
  for block in payload.content array:
     switch block.type:
       "text" -> content_parts.push(Text(block.text))
       "tool_use" -> content_parts.push(ToolCall{id:block.id, name:block.name, arguments_json:block.input})
       "thinking" -> content_parts.push(Thinking{text:block.thinking, provider:"anthropic"})
       "redacted_thinking" -> content_parts.push(Thinking{text:"<redacted>", provider:"anthropic"} + warn)
       default -> content_parts.push(Text(stringify(block)) + warn)

  usage = map_usage(payload.usage)

  structured_output = null
  if requested_response_format != Text:
      structured_output = try_parse_json_outputs_or_best_effort(content_parts, stop_reason)

  return ProviderResponse{
     output: { content: content_parts, structured_output },
     usage,
     cost: None,
     provider: Anthropic,
     model,
     raw_provider_response: None,
     finish_reason,
     warnings
  }
```

**J. Edge case matrix**

| Case | Claude wire shape (high-level) | Canonical result | FinishReason | Validation outcome |
|---|---|---|---|---|
| Text-only response | `content=[{type:"text"}]`, `stop_reason=end_turn` citeturn1view0turn7view3 | `output.content=[Text]` | `Stop` | OK |
| Tool-only response | `content=[{type:"tool_use",...}]`, `stop_reason=tool_use` citeturn7view0turn7view3 | `output.content=[ToolCall]` | `ToolCalls` | OK |
| Text + tool_use same response | `content=[{text...},{tool_use...}]` citeturn9view5 | `[Text, ToolCall]` preserving order | `ToolCalls` | OK |
| Multiple tool_use blocks | `content=[..., {tool_use...}, {tool_use...}]` with parallel calling default citeturn9view5turn7view0 | multiple `ToolCall` parts in order | `ToolCalls` | OK |
| Thinking blocks present | `content=[{thinking...}, {text...}]` (if enabled) citeturn17view3turn7view1 | `Thinking(provider="anthropic")` + `Text` | depends on stop_reason | OK + warning for redacted thinking |
| Truncated output | `stop_reason=max_tokens` citeturn7view3 | text/tool blocks decoded as present | `Length` | OK; structured_output parsing likely fails → warning citeturn18view5 |
| Stop sequence triggered | `stop_reason=stop_sequence`, `stop_sequence="<x>"` citeturn6view2turn7view3 | `Text` (and/or others) | `Stop` | OK |
| Refusal | `stop_reason=refusal` (may still be 200) citeturn7view3turn18view5 | refusal text in `Text` blocks; `structured_output=None` | `ContentFilter` | OK + warning recommended |
| Pause turn | `stop_reason=pause_turn` citeturn7view3turn9view2 | partial content preserved | `Other` | OK + warning recommended |
| Error returned HTTP | HTTP status != 200 with error JSON `{type:"error", error:{type,message}, request_id}` citeturn1view1 | `ProviderError::Status` / `CredentialsRejected` | `Error` (via error path) | Adapter-level mapping required |
| Empty output | `content=[]` but usage present (output_tokens may still be non-zero) citeturn21view2turn17view3 | `output.content=[]`, `structured_output=None` | map stop_reason | OK + warning `empty_output` |
| Usage missing/partial | `usage` missing or fields missing (docs allow expansion) citeturn1view1turn21view2 | `Usage` fields as None when absent | map stop_reason | OK + warning `usage_missing` |

The Anthropic adapter is complete when all canonical semantics are preserved without transport leakage and the translator contract is fully satisfied.
