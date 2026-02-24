# OpenAI Responses API Translator Design for provider_runtime

Specialist perspective: I’m approaching this as an API integration architect focused on LLM transport adapters and canonicalization layers.

This document specifies a complete adapter translation surface for mapping your canonical `ProviderRequest`/`ProviderResponse` types to/from the OpenAI Responses API, strictly within the boundaries of your crate-private translator contract. It is grounded only in your repository’s canonical types + translator contract and official OpenAI documentation:
- https://platform.openai.com/docs/api-reference/responses/create
- https://platform.openai.com/docs/api-reference/responses/object
- https://platform.openai.com/docs/guides/function-calling
- https://platform.openai.com/docs/guides/structured-outputs

## Translator contract alignment

### Contract surface

Your translator surface is exactly the crate-private `ProviderTranslator` trait with two required methods and two associated payload types.

**Associated types**
- `type RequestPayload;` — provider protocol payload for outbound encoding.
- `type ResponsePayload;` — provider protocol payload for inbound decoding.

**Required methods**
- `encode_request(&self, req: &ProviderRequest) -> Result<RequestPayload, ProviderError>`
- `decode_response(&self, payload: &ResponsePayload) -> Result<ProviderResponse, ProviderError>`

Stage 14 explicitly commits that the OpenAI translator must implement canonical-to-OpenAI and OpenAI-to-canonical translation *via this translator contract* and must provide deterministic encode/decode behavior with stable warnings and deterministic errors for unsupported intent (no silent lossy conversion).

### Responsibilities, invariants, guarantees, errors (per method)

#### encode_request responsibilities

Encode canonical semantics inside `ProviderRequest` into a single OpenAI Responses API `POST /v1/responses` JSON request body (as `RequestPayload`).

**Inputs**: `ProviderRequest` canonical fields include:
- `model: ModelRef { provider_hint?: ProviderId, model_id: String }`
- `messages: Vec<Message { role: MessageRole, content: Vec<ContentPart> }>`
- `tools: Vec<ToolDefinition>`
- `tool_choice: ToolChoice`
- `response_format: ResponseFormat`
- optional sampling and limits: `temperature`, `top_p`, `max_output_tokens`
- `stop: Vec<String>`
- `metadata: BTreeMap<String, String>`

**Input invariants (must validate, else deterministic error)**
- Provider hint mismatch: If `req.model.provider_hint` is present and not `Openai`, encoding must fail with a deterministic `ProviderError::Protocol` (this is canonical semantic mismatch at the adapter boundary).
- `model_id` must be non-empty (OpenAI requires a model ID string).
- Messages must be representable in OpenAI’s `input` item model:
  - All non-tool, non-reasoning content must be expressible as OpenAI text input parts (`input_text`).
  - Tool calls and tool results must match the OpenAI “function_call” and “function_call_output” item flow (see tool semantics section); otherwise error.
- `metadata` must comply with OpenAI metadata constraints (max 16 keys, key length ≤ 64, value length ≤ 512). If the canonical payload violates constraints, encoding must fail (do not silently truncate, per Stage 14 no silent lossy).
- `stop` is canonical but not documented as supported by Responses API request parameters; encoding must take a deterministic “unsupported intent” path (see Canonical gaps section).

**Output guarantees**
- Encoded payload contains only OpenAI request fields; no canonical-only fields are leaked to OpenAI.
- Equal canonical input must encode identically (stable ordering and stable normalization).

**Error conditions**
- Return `ProviderError::Protocol` for canonical intent that cannot be represented without silent loss (e.g., `stop`, non-text content parts, illegal tool result structure, tool choice “Specific” without a resolvable function name).
- Return `ProviderError::Serialization` if canonical JSON fields cannot be serialized into the OpenAI payload deterministically (should be rare with `serde_json::Value` schemas).

#### decode_response responsibilities

Decode an OpenAI Responses API response payload into a canonical `ProviderResponse`. OpenAI response objects include `status`, `output` array, `usage`, and may include `error` and `incomplete_details`.

**Input invariants**
- Payload must be an OpenAI “Response object” shape when HTTP succeeded, or a provider error shape where `error` is present (including streamed `response.failed` events).
- Decoder must not assume the first `output` item is an assistant message; OpenAI explicitly notes `output` ordering/length depends on model response, and SDKs only *sometimes* expose convenience `output_text`. Decoder must iterate items and types deterministically.

**Output guarantees**
- Populate `ProviderResponse.provider = ProviderId::Openai` and `ProviderResponse.model` from response `model`.
- Populate `ProviderResponse.output.content` as a canonical `AssistantOutput` sequence that preserves order deterministically.
- Populate `ProviderResponse.finish_reason` deterministically from OpenAI `status` + `incomplete_details.reason` (matrix below).
- Populate `usage` fields from OpenAI `usage` if present; otherwise return `Usage::default()` and emit a stable warning when absent.
- `raw_provider_response` must be `None` (Stage 14: keep provider raw JSON internal to adapter layer).

**Error conditions**
- If OpenAI response indicates protocol failure (e.g., `status: "failed"` with `error` object), return `Err(ProviderError::Status|Protocol)` rather than a “successful” `ProviderResponse`.
- If payload cannot be parsed deterministically because of missing required fields or unknown item types that cannot be represented without silent loss, return `Err(ProviderError::Protocol)` (with stable error message).

## Canonical to OpenAI request mapping

### OpenAI request form selection

OpenAI Responses API “Create a model response” accepts `model` and `input` (string or array), plus optional configuration fields like `instructions`, `max_output_tokens`, `temperature`, `top_p`, `tool_choice`, `tools`, `metadata`, and streaming controls.

This adapter must always encode `input` as an **array** (even if there is a single user message) to support canonical multi-message semantics and to support tool call / tool output items deterministically. OpenAI explicitly supports `input` as an array of items.

### Field-by-field mapping

The table below is the normative mapping. All invalid-state behavior is deterministic error (or deterministic warning where explicitly stated as “lossy but allowed”) per Stage 14.

#### ModelRef

| Canonical | OpenAI field | Rules | Validation / invalid states |
|---|---|---|---|
| `req.model.model_id` | `model` | Copy verbatim. | Empty → `ProviderError::Protocol` (“missing model_id”). |
| `req.model.provider_hint` | *(no OpenAI field)* | Validate only: if present must equal `Openai`; else error. | Any non-Openai hint → error (prevents routing/provider leakage). |

#### Messages, roles, and content parts

OpenAI message items in the Responses API input list use a `role` and `content`, where content items include `"type": "input_text"` for text inputs.

Canonical message roles:
- `System`, `User`, `Assistant`, `Tool`

**Role mapping strategy**
- Encode every canonical `Message` as either:
  - an OpenAI input item of type `"message"` (for `System`, `User`, `Assistant`) with `role` set accordingly, containing only text input parts; OR
  - an OpenAI input item of type `"function_call_output"` (for `Tool` role messages that represent tool results), because Responses API tool outputs are provided as distinct items correlated by `call_id`.

**Canonical → OpenAI “role/content-type” normalization**
- OpenAI uses `input_text` for *input* message content parts and `output_text` for *output* message content parts; canonical uses `ContentPart::Text` both ways. The adapter must absorb this transport difference.

##### ContentPart mapping (input)

| Canonical ContentPart | OpenAI input representation | Transformation rules | Invalid / edge conditions |
|---|---|---|---|
| `Text { text }` | message content item `{ "type":"input_text", "text": <text> }` | Preserve exact text. | Empty text allowed but discouraged; if *all* content parts are empty across all messages → error (“empty input”), since OpenAI input would be vacuous. (No explicit OpenAI spec line; implement as canonical invariant to avoid undefined provider behavior.) |
| `Thinking { text, provider }` | **Not transmitted** by default | Canonical “Thinking” is runtime-level; sending it back would leak chain-of-thought. Encode-time policy: drop from OpenAI request and emit deterministic warning `dropped_thinking_on_encode` only if any Thinking parts exist. (Lossy is explicit + warned.) |
| `ToolCall { tool_call }` | input item `{ "type":"function_call", "id":?, "call_id":?, "name":..., "arguments": ... }` | **Primary strategy**: only allow ToolCall content parts in `Assistant` role messages. Represent them as standalone `function_call` items in `input` array, not nested within a message. `arguments_json` must be serialized to a JSON string for OpenAI `arguments`. | If ToolCall appears in non-Assistant role message → error. If `arguments_json` cannot be serialized to JSON text → `ProviderError::Serialization`. |
| `ToolResult { tool_result }` | input item `{ "type":"function_call_output", "call_id": ..., "output": ... }` | Only allow ToolResult in `Tool` role messages (or in a tool-only Message). Convert tool_result.content to a string payload (rules below). Map `tool_call_id` → `call_id`. | If function output is non-text / multi-part non-text → error; Responses API supports non-string outputs for images/files, but canonical cannot represent; treat as unsupported canonical content. |

##### Multi-part content handling (input)

Canonical messages contain `Vec<ContentPart>`. OpenAI message content supports arrays of content items; for our constrained adapter (text-only + tool flow), map:
- multiple `Text` parts → multiple `input_text` parts in the same OpenAI message (preserve order).
- mixed `Text` + tool parts → **split** into separate OpenAI input items in stable order:
  - message item containing all consecutive `Text` parts
  - followed by one `function_call` item per ToolCall part
  - followed by `function_call_output` item(s) only if the canonical message role is `Tool` and contains ToolResult parts.

This splitting prevents leaking OpenAI-specific “tool role messages” patterns upward and keeps canonical semantics consistent: tools are distinct steps correlated by `call_id`.

##### Tool result output-string serialization rules (canonical → OpenAI)

OpenAI states: “The result you pass in the `function_call_output` message should typically be a string … the format is up to you (JSON, error codes, plain text, etc.).”

Deterministic adapter rule for producing `output`:
- If `tool_result.content` contains exactly one `ContentPart::Text { text }`, set `output = text` verbatim.
- If it contains multiple `Text` parts, `output = join(texts, "\n")` in order (deterministic).
- Any `Thinking`, `ToolCall`, or nested `ToolResult` inside a tool result → `ProviderError::Protocol` (“invalid tool_result.content for OpenAI function_call_output”).

#### ToolDefinition mapping

Canonical tool definition:
- `ToolDefinition { name, description?: Option<String>, parameters_schema: Value }`

OpenAI tool definitions in function calling:
- `tools` is an array where function tools are represented with `type: "function"` and function schema fields; the guide shows both nested and flattened forms, and documents strict mode for schema adherence.

**Normative mapping decision (to minimize ambiguity + keep schema close to canonical)**  
Encode tools using the *flattened function tool form*:

```text
{
  "type": "function",
  "name": <canonical.name>,
  "description": <canonical.description or omitted>,
  "parameters": <canonical.parameters_schema>,
  "strict": <computed boolean; see below>
}
```

This matches the function-calling guide’s examples that include `type`, `name`, `description`, `parameters`, and optionally `strict`.

**Strict mode computation (deterministic, best-effort)**
OpenAI: strict mode “works by leveraging structured outputs” and requires:
1) `additionalProperties` must be `false` for each object in `parameters`
2) All fields in `properties` must be listed in `required`
Optional fields can be represented by `null` in the `type` union.

Adapter behavior:
- If `parameters_schema` is *strict-compatible* by validation (recursive algorithm below), set `"strict": true`.
- Otherwise **omit** `"strict"` (or set `false`) and emit a stable warning `tool_schema_not_strict_compatible_strict_disabled` containing the OpenAI requirements. This is lossy only in the sense of not enabling strict; schema itself is still passed.

**Tool definition validation**
- `name` must be non-empty; else `ProviderError::Protocol`.
- `parameters_schema` must be JSON object schema-ish (at minimum, JSON object value). If not, still pass-through because OpenAI accepts JSON schemas, but if it’s not a JSON object at all, treat as protocol error to avoid provider rejection surprises.

#### ToolChoice mapping

Canonical `ToolChoice`: `None | Auto | Required | Specific` with default `Auto`.

OpenAI `tool_choice` can be:
- `"auto"` (default): call zero/one/multiple functions
- `"required"`: call one or more functions
- `"none"`: call no tools (imitates passing no functions)
- forced function: `{ "type": "function", "name": "<function_name>" }`
- allowed tools: `{ "type":"allowed_tools", "mode":"auto", "tools":[...] }`

Normative mapping:

| Canonical ToolChoice | OpenAI tool_choice | Rules / validation |
|---|---|---|
| `None` | `"none"` | Even if tools are present, force none. |
| `Auto` | `"auto"` | Default behavior. |
| `Required` | `"required"` | Requires at least one tool invocation. |
| `Specific { name }` | `{ "type":"function", "name":"<name>" }` | Validate that `<name>` matches a declared tool definition; otherwise return deterministic protocol error. |

#### ResponseFormat mapping

Canonical `ResponseFormat`:
- `Text`
- `JsonObject`
- `JsonSchema { name, schema }`

OpenAI:
- Structured Outputs via Responses API uses `text.format`:
  - JSON mode: `text.format = { "type": "json_object" }`
  - JSON schema: `text.format = { "type": "json_schema", "name": ..., "schema": ..., "strict": true }` (guide shows strict true in examples).

Normative mapping:

| Canonical ResponseFormat | OpenAI request field | Rules / validation |
|---|---|---|
| `Text` | `text: { format: { type: "text" } }` | Set explicitly to avoid provider-default drift. |
| `JsonObject` | `text: { format: { type: "json_object" } }` | **Must** ensure “JSON” appears somewhere in the input context; OpenAI says API throws error if “JSON” is absent when JSON mode is enabled. Adapter must validate and fail fast if missing (no silent mutation). |
| `JsonSchema{name,schema}` | `text: { format: { type: "json_schema", name, schema, strict: true } }` | Set `strict: true` per guide examples for schema adherence. If OpenAI rejects schema features, surface as provider status/protocol error. |

#### Sampling and limits

OpenAI request body supports:
- `temperature` (0–2) and recommends altering temperature or `top_p` but not both.
- `top_p` default 1.
- `max_output_tokens`: upper bound including visible output tokens and reasoning tokens.

Mapping:
- `temperature` → `temperature` if `Some`
- `top_p` → `top_p` if `Some`
- `max_output_tokens` → `max_output_tokens` if `Some`

Validation:
- temperature must be within the documented 0–2 range; else error.
- top_p should be within [0,1]; OpenAI describes it as probability mass; enforce (error if out of range) to avoid ambiguous provider behavior.
- If both temperature and top_p are supplied, preserve both (canonical intent), but emit warning `both_temperature_and_top_p_set` because OpenAI recommends not altering both.

#### Max token handling vs truncation

OpenAI supports `truncation` strategy (`auto` vs `disabled`) to handle context-window overflow. Canonical has no field for truncation policy. Deterministic policy: do not set `truncation` (allow OpenAI default `disabled` as documented) and let overflow become an error rather than silently dropping context.

#### Stop sequences

Canonical: `stop: Vec<String>`.  
OpenAI Responses API reference does not document a `stop` request parameter.

Deterministic policy: **encoding fails** with `ProviderError::Protocol` (“stop sequences unsupported by OpenAI Responses API in documented surface”), unless you extend canonical to represent provider-specific “stop-like” behavior elsewhere (not recommended).

#### Metadata

Canonical `metadata: BTreeMap<String,String>` maps to OpenAI `metadata` map. OpenAI constrains it to 16 key-value pairs, key length ≤ 64, value length ≤ 512.

Deterministic validation:
- If >16 entries: error (no truncation).
- If any key/value exceeds limits: error (no truncation).
- Else pass through.

## OpenAI response to canonical mapping

### High-level decoding goals

OpenAI response object includes:
- `status` one of `completed`, `failed`, `in_progress`, `cancelled`, `queued`, `incomplete`
- `output`: array of items; ordering depends on model response.
- `usage` object may be null (especially in streaming/cancelled cases).
- `incomplete_details` may include reason (seen: `max_output_tokens`).
- `error` may be present for failed responses.

Canonical output expects:
- `ProviderResponse.output: AssistantOutput { content: Vec<ContentPart>, structured_output?: Value }`
- `ProviderResponse.finish_reason: FinishReason`
- `ProviderResponse.usage: Usage`

### Deterministic parsing algorithm (non-streaming)

Below is normative pseudocode (not Rust) for `decode_response`.

#### decode_response(payload) pseudocode

```text
function decode_response(payload):
  # 0) Validate top-level shape minimally
  if payload is not an object:
    return Err(ProviderError::Protocol(provider=Openai, message="non-object response payload"))

  # 1) Extract common fields
  status = payload["status"] as string?   # may be absent in some error shapes
  model  = payload["model"]  as string?   # required for normal response
  error_obj = payload["error"]            # may be null or object

  # 2) Hard error handling (provider protocol error payload)
  if error_obj is object:
    code = error_obj["code"] as string? (best-effort)
    msg  = error_obj["message"] as string? (fallback to stringified error_obj)
    # Choose ProviderError classification:
    # - If HTTP status code is known externally, prefer ProviderError::Status.
    # - In translator-only context, treat as ProviderError::Protocol because it's a provider-declared failure.
    return Err(ProviderError::Protocol(provider=Openai, model=model?, request_id=None, message=fmt("openai error: {code}: {msg}")))

  if status == "failed":
    # streaming docs show failed responses carry error object; if not, still error out deterministically
    return Err(ProviderError::Protocol(provider=Openai, model=model?, message="openai status=failed"))

  # 3) Parse output items
  output_items = payload["output"] as array? else []  # missing => treat as empty
  content_parts = []
  tool_calls_seen = map call_id -> ToolCall (for validation only)

  for item in output_items in order:
    t = item["type"] as string?
    if t == "message":
      role = item["role"] as string?
      # Only treat assistant role messages as assistant output
      # Parse content array:
      for part in item["content"] as array?:
        ptype = part["type"]
        if ptype == "output_text":
          text = part["text"] as string default ""
          if text != "":
            append ContentPart::Text{text} to content_parts
        else:
          # refusal content or other parts: map as Text with stable warning OR error if cannot preserve meaning
          # (see refusal mapping below)
          handle_non_output_text_part(part)
    else if t == "function_call":
      # per function calling guide: output entries include call_id, name, arguments string
      call_id = item["call_id"] as string? else error
      name = item["name"] as string? else error
      args_str = item["arguments"] as string? else error
      args_json = parse_json(args_str) else:
                  # deterministic fallback: store as JSON string (so canonical still has arguments_json)
                  args_json = args_str as JSON string value
                  add warning "tool_arguments_invalid_json"
      tool_call = ToolCall{id=call_id, name=name, arguments_json=args_json}
      append ContentPart::ToolCall{tool_call} to content_parts
      tool_calls_seen[call_id] = tool_call
    else if t == "reasoning":
      # If present, normalize to ContentPart::Thinking
      text = extract reasoning text/summary best-effort
      if text not empty:
        append ContentPart::Thinking{text=text, provider=Openai} to content_parts
    else:
      # Unknown output item type. Deterministic choice:
      return Err(ProviderError::Protocol(provider=Openai, model=model?, message="unsupported output item type: " + t))

  # 4) Structured output extraction
  structured_output = maybe_parse_structured_output(payload, content_parts)

  # 5) Usage mapping
  usage = decode_usage(payload["usage"]) with warning if missing/null

  # 6) Finish reason mapping from status + incomplete_details
  finish_reason = map_finish_reason(status, payload["incomplete_details"], content_parts)

  # 7) Assemble canonical ProviderResponse
  return Ok(ProviderResponse{
    output: AssistantOutput{content=content_parts, structured_output=structured_output},
    usage: usage,
    cost: None,
    provider: Openai,
    model: model or "<unknown-model>",
    raw_provider_response: None,
    finish_reason: finish_reason,
    warnings: warnings
  })
```

This algorithm is directly grounded in OpenAI’s `status`, `output` array semantics, and function calling output item structure including `type: "function_call"`, `call_id`, `name`, and JSON-encoded `arguments`.

### Deterministic ordering and “combine multiple output blocks”

OpenAI states order/length of `output` depends on model response; therefore canonical must preserve OpenAI’s item order deterministically.

Canonical combination rules:
- `output.message` items with `output_text` parts become `ContentPart::Text` entries in canonical, in the exact order encountered.
- `function_call` items become `ContentPart::ToolCall` entries in canonical, in the exact order encountered.
- If OpenAI interleaves text and tool calls, canonical content interleaves them in the exact same order (no reordering).

### Handling refusal content

Streaming docs define `response.refusal.delta/done` events for refusal text.  
The non-streaming response format for refusal is not specified in your allowed sources, so any mapping beyond “preserve refusal text” is ambiguous.

Deterministic mapping:
- If refusal text is present (via a known content part type in non-streaming payloads, or inferred from streaming accumulation), convert it to `ContentPart::Text` and emit warning `model_refusal` with message “OpenAI refusal text present; semantics mapped to Text”.
- Finish reason mapping: prefer `FinishReason::ContentFilter` only when OpenAI explicitly signals content filtering/incompletion reason; otherwise keep `FinishReason::Other` with warning (see finish matrix and ambiguity notes).

### Handling reasoning items

OpenAI streaming includes `response.reasoning_text.delta/done` and `response.reasoning_summary_text.delta/done`.  
Responses API reference also indicates reasoning configuration and that reasoning outputs may exist as “reasoning item outputs” (via `include` options).

Deterministic mapping:
- Any reasoning text or reasoning summary extracted from output is normalized into canonical `ContentPart::Thinking { text, provider: Some(Openai) }`.
- Do not attempt provider-specific distinctions (summary vs full reasoning) in canonical state (strict no leakage).

## Structured output handling

### When structured output is expected

Canonical expects structured output when `ProviderRequest.response_format` is `JsonObject` or `JsonSchema`.

OpenAI enables:
- JSON mode using `text.format = { "type": "json_object" }` for Responses API.
- JSON Schema mode using `text.format = { type: "json_schema", name, schema, strict: true }`.

### Detecting “JSON mode” vs “JSON schema mode”

Detection is based on the request you encoded (canonical intent), not solely on the response payload, because OpenAI response does not reliably echo the exact `text.format` fields in all views and your translator has to avoid OpenAI-specific leakage.

Normative approach:
- If canonical request was `JsonSchema`, attempt strict JSON parsing and set `structured_output` only if parse succeeds.
- If canonical request was `JsonObject`, parse as JSON object and set `structured_output` only if parse succeeds.

### Safe extraction and malformed JSON

OpenAI explicitly warns JSON mode is valid JSON “except for some edge cases that you should detect and handle,” including incomplete outputs.

Canonical policies:
- Always preserve raw assistant output as `ContentPart::Text` (even if it’s a JSON string), so callers can log/debug without provider leakage.
- If JSON parse fails:
  - Set `AssistantOutput.structured_output = None`.
  - Emit warning `structured_output_parse_failed` with a deterministic message and (optionally) a short excerpt length, not the full payload.
  - Do **not** fail the whole decode unless the caller demanded hard schema conformance at a different layer (canonical types do not have such a flag today).

### JSON-mode-specific validation hazard (required “JSON” keyword)

OpenAI: when JSON mode is enabled, the API will throw an error if the string “JSON” does not appear somewhere in the context, to prevent runaway whitespace responses.

Translator invariant:
- For canonical `JsonObject`, validate the messages contain substring “JSON” somewhere in any text content before encoding. If absent, return `ProviderError::Protocol` with stable message requiring the caller to include such instruction.

## Tool calling semantics

### OpenAI tool call correlation and canonical IDs

OpenAI Responses function calling output items include:
- `type: "function_call"`
- `call_id` used later to submit function result
- `name`
- JSON-encoded `arguments`

Canonical tool call uses:
- `ToolCall { id: String, name: String, arguments_json: Value }`

Normative mapping for determinism and correlation:
- Canonical `ToolCall.id` **must equal OpenAI `call_id`** (not OpenAI’s other `id` field) because OpenAI explicitly uses `call_id` to correlate tool calls with `function_call_output`.

This prevents OpenAI transport leakage (call_id vs id) by collapsing correlation onto one canonical identifier.

### Argument parsing rules

- OpenAI `arguments` arrives as a JSON-encoded string.
- Decoder must `JSON.parse(arguments)` into canonical `arguments_json`.
- If parsing fails, canonical still requires a JSON `Value`. Deterministic fallback:
  - Set `arguments_json` to a JSON string value of the raw arguments text.
  - Emit warning `tool_arguments_invalid_json`.  
This avoids silent loss and preserves original bytes.

### Multiple tool calls

OpenAI may return multiple `function_call` items in one response output array.

Canonical output rules:
- Decode each into `ContentPart::ToolCall` in output order.
- Set `FinishReason::ToolCalls` if the response ends with tool calls and contains no final assistant text (see finish mapping policy below).

### Text + tool call coexistence

OpenAI can interleave message output text and tool calls (since `output` array order is model-dependent).

Canonical must:
- Preserve output order: text parts and tool calls appear in the same sequence.
- FinishReason is determined by overall response status and incompleteness, not by mere presence of tool calls (see matrix).

### Tool result appears without matching call_id

In canonical encoding, if you attempt to send `ToolResult.tool_call_id` without ever including a corresponding tool call item in the `input` list, OpenAI might still accept it (not specified), but it is semantically risky and could cause model confusion.

Deterministic validation rule (encode-time):
- If the request contains any `ToolResult` items, their `tool_call_id` must match some earlier tool call ID present in the same request’s message history (either in canonical messages as ToolCall parts or already-decoded tool calls passed back). If not, fail with `ProviderError::Protocol("tool_result_without_matching_tool_call")`.

This preserves canonical semantics and prevents silent behavior differences.

### Streaming considerations for tool calls

OpenAI streaming emits:
- `response.output_item.added/done` events to indicate new output items and their completion.
- `response.function_call_arguments.delta/done` events for incremental tool argument construction.

Design-level streaming implication:
- A tool call item becomes semantically “stable” only after the `.done` event for arguments; callers must treat `.delta` as partial and non-JSON-safe.

## Finish reason mapping matrix

Canonical finish reasons are: `Stop | Length | ToolCalls | ContentFilter | Error | Other`.  
OpenAI Responses API provides top-level `status` and, when incomplete, `incomplete_details.reason` (example: `max_output_tokens`).

### Normative matrix (including error-return behavior)

| OpenAI status | incomplete_details.reason | Canonical FinishReason | Decode behavior |
|---|---|---|---|
| `completed` | null | `Stop` | Return `Ok(ProviderResponse)` with parsed output. |
| `incomplete` | `max_output_tokens` | `Length` | Return `Ok` with partial output (if any) + warning `openai_incomplete_max_output_tokens`. |
| `incomplete` | unknown / undocumented | `Other` | Return `Ok` + warning `openai_incomplete_unknown_reason:<value>`. |
| `failed` | any | `Error` | **Return `Err(ProviderError::Protocol/Status)`** (do not return ProviderResponse). |
| `cancelled` | null/any | `Other` | Default: return `Err(ProviderError::Protocol)` because operation not completed; if you choose to surface partial output, must warn. (OpenAI shows cancelled with partial output + usage null.) |
| `in_progress` / `queued` | n/a | `Other` | In non-streaming decode, treat as protocol error (“unexpected nonterminal status in final payload”) unless your transport layer is polling/calling decode mid-flight. |

### Tool invocation finish semantics

Responses API does not expose a `finish_reason` token like Chat Completions; tool invocation must be inferred from the final output content. The function calling guide indicates the response `output` array contains `function_call` entries.

Deterministic rule to set `FinishReason::ToolCalls`:
- If status is `completed` **and** the final output content includes one or more tool calls **and** there is no assistant text output after the last tool call, set `FinishReason::ToolCalls` instead of `Stop`. This preserves canonical semantics that a tool request is a “pause for tool execution.”

If text follows tool calls in the same response, keep `Stop` (the model completed a response that includes tool calls and final text).

### Content filter finish semantics

OpenAI Responses documents `content_filter` as an `incomplete_details.reason` value.

Deterministic mapping behavior:
- If `status=incomplete` and `incomplete_details.reason=content_filter`, map to `FinishReason::ContentFilter`.
- If refusal text appears without explicit `content_filter`, preserve refusal as text and warn without forcing `ContentFilter`.

## Usage mapping

Canonical `Usage` fields:
- `input_tokens`, `output_tokens`, `total_tokens`, `reasoning_tokens`, `cached_input_tokens` (all optional).

OpenAI Responses `usage` includes:
- `input_tokens`
- `input_tokens_details.cached_tokens`
- `output_tokens`
- `output_tokens_details.reasoning_tokens`
- `total_tokens`

Normative mapping:

| OpenAI usage field | Canonical Usage field | Notes |
|---|---|---|
| `usage.input_tokens` | `input_tokens` | direct copy |
| `usage.output_tokens` | `output_tokens` | direct copy |
| `usage.total_tokens` | `total_tokens` | direct copy; canonical has `derived_total_tokens()` fallback. |
| `usage.output_tokens_details.reasoning_tokens` | `reasoning_tokens` | direct copy when present |
| `usage.input_tokens_details.cached_tokens` | `cached_input_tokens` | direct copy when present |

Fallback behavior:
- If OpenAI `usage` is null/absent (examples show `usage: null` in cancelled and streaming-created objects), return `Usage::default()` and emit warning `usage_missing`.

## Streaming design

No implementation code here—this is the design-level accumulation model. It is constrained to OpenAI’s SSE event types and Stage 14 determinism rules.

### OpenAI streaming event primitives relevant to canonical mapping

OpenAI emits:
- lifecycle: `response.created`, `response.in_progress`, `response.completed`, `response.failed`, `response.incomplete`
- output item boundaries: `response.output_item.added`, `response.output_item.done`
- text deltas: `response.output_text.delta`, `response.output_text.done`
- refusal deltas: `response.refusal.delta`, `response.refusal.done`
- function argument deltas: `response.function_call_arguments.delta`, `response.function_call_arguments.done`
- reasoning deltas: `response.reasoning_text.delta/done`, reasoning summary deltas: `response.reasoning_summary_text.delta/done`
- terminal error event: `error`

### Chunk accumulation model (canonical view)

Maintain an in-memory “response assembly state” keyed by OpenAI’s `(output_index, item_id)` from streaming events.

Deterministic state:
- `items[output_index] => {type, role?, content_parts[], function_call_args_buffer, refusal_buffer, reasoning_buffers...}`

Accumulation logic:
- On `response.output_item.added`: allocate `items[output_index]` with the given initial item shape (e.g., message with empty content).
- On `response.output_text.delta`: append to a buffer for `(output_index, content_index)`; do not emit canonical `Text` yet if you require “finalize only” policy.
- On `response.output_text.done`: finalize buffer into a stable text string; produce canonical `ContentPart::Text` at the correct position within the output ordering.
- On `response.function_call_arguments.delta`: append to function args buffer for the item; arguments may be partial and must not be parsed.
- On `response.function_call_arguments.done`: finalize args string; now parse JSON deterministically into canonical `ToolCall.arguments_json` with the fallback rule if parsing fails; emit canonical `ToolCall` with `id` to be determined (see below).
- On `response.refusal.*`: treat similarly to output_text but store as refusal; map to canonical Text with warning at finalization.
- On `response.reasoning_text.*` and `response.reasoning_summary_text.*`: finalize into canonical `Thinking` parts (provider Openai).
- On terminal lifecycle:
  - `response.completed` → run the same deterministic “non-streaming” decode over the assembled final object (or directly over the `response` object provided in the event) and produce canonical completion.
  - `response.incomplete` → produce canonical response with `FinishReason::Length` if reason is `max_output_tokens`, else `Other`, using matrix.
  - `response.failed` or `error` → translate into `ProviderError` (no ProviderResponse).

### ToolCall.id determinism in streaming mode

Because streaming function-call events don’t expose `call_id` in the event types shown (your allowed sources document argument events keyed by `item_id`), but definitive correlation in Responses is via `call_id` (from `function_call` items), the streaming adapter must defer canonical ToolCall emission until it has the actual `function_call` item fields (likely delivered via `response.output_item.done`’s item snapshot or other item payload).

If a streamed “done” item snapshot contains `call_id`, then canonical `ToolCall.id = call_id`. If not present, treat as protocol error (cannot correlate tool output safely).

### Race conditions and partial-tool-call scenarios

Documented hazards:
- Argument deltas can arrive before “done,” and may not parse as JSON until finalization.
- Output items are appended over time; you must preserve `output_index` ordering.

Deterministic mitigations:
- Never attempt to parse arguments until `.done`.
- Never “reorder” based on item type; always follow `output_index` and event `sequence_number` ordering.
- Canonical completion signal is only emitted upon lifecycle terminal event (`response.completed` or `response.incomplete`)—never before.

## Canonical gaps and required changes

Stage 14 requires identifying canonical mismatches and proposing minimal changes without provider leakage.

### ToolChoice::Specific mapping status

Canonical now uses `ToolChoice::Specific { name: String }`.
OpenAI forced-tool mode maps directly to `{ "type": "function", "name": "<function_name>" }`.

Adapter behavior:
- Map `ToolChoice::Specific { name }` → OpenAI `tool_choice = { "type": "function", "name": name }`.
- Validate the tool name exists in the declared tool definitions; return deterministic protocol error on mismatch.

### Canonical stop sequences cannot be expressed

Canonical includes `stop: Vec<String>`.  
OpenAI Responses API reference does not document a `stop` request parameter.

Options:
- Do **not** change canonical: treat as OpenAI capability gap and fail encoding deterministically when `stop` is set (recommended; avoids provider leakage).
- Avoid adding OpenAI-specific stop fields to canonical (would violate strict invariants).

### Canonical does not model OpenAI-specific truncation/parallel-tool toggles

OpenAI supports:
- `parallel_tool_calls` to control multiple tool calls.
- `truncation` policy.

Canonical lacks such knobs. No change required unless you want cross-provider support; for now, keep defaults and maintain determinism.

## Strict invariants to prevent provider leakage

These are non-negotiable adapter rules derived from your Stage 14 constraints and canonical schema goals.

### Invariants

- Canonical structures must never include OpenAI enum names or wire fields (e.g., `output_text`, `input_text`, `call_id`, `function_call_output`). They must only use canonical enums (`ContentPart`, `ToolCall`, `ToolResult`, etc.).
- All OpenAI transport distinctions (input vs output part types, status vs finish, call_id vs id) are absorbed entirely inside the translator.
- Equal canonical input encodes identically; equal provider payload decodes identically (ordering, stringification, normalization).
- Unsupported canonical intent must not be silently dropped. The translator must:  
  - either error deterministically (`ProviderError::Protocol`), or  
  - drop with an explicit stable warning code/message if and only if Stage 14 allows lossy with warnings and you’ve documented it (e.g., dropping Thinking on encode).
- `raw_provider_response` remains `None` by default (no provider payloads in canonical responses).
- ToolChoice::Specific must not degrade silently: encode with the explicit canonical tool name and fail deterministically on name mismatch.
- All OpenAI statuses must map deterministically: terminal statuses are handled; nonterminal statuses in final decode are treated as protocol errors.

## Edge case matrix

This table enumerates the required edge cases with OpenAI shape, canonical output, finish reason, and validation outcome. Shapes are expressed using the OpenAI terms documented in your allowed sources.

| Case | OpenAI shape (high-level) | Canonical result | FinishReason | Validation outcome |
|---|---|---|---|---|
| Text-only response | `status=completed`, `output=[{type:"message", role:"assistant", content:[{type:"output_text", text:...}]}]` | `AssistantOutput.content=[Text(...)]` | `Stop` | OK |
| Tool-only response | `status=completed`, `output=[{type:"function_call", call_id,...}]` | `content=[ToolCall(id=call_id,...)]` | `ToolCalls` (per rule: no trailing text) | OK |
| Text + tool response | `output` includes message + function_call items interleaved | `content` preserves order: `Text`, `ToolCall`, `Text` ... | Typically `Stop` if text ends; else `ToolCalls` | OK |
| Multiple tool calls | `output=[{function_call...}, {function_call...}, ...]` | Multiple `ToolCall` parts in order | `ToolCalls` | OK |
| Reasoning-only output | streaming includes `response.reasoning_text.*` without output_text | `content=[Thinking(provider=Openai,...)]` | `Stop` (if completed) | OK + warning optional (“no user-visible text”) |
| Truncated output | `status=incomplete`, `incomplete_details.reason="max_output_tokens"` | partial `Text`/others decoded if present | `Length` | OK + warning `openai_incomplete_max_output_tokens` |
| Content filtered / refusal | streaming shows `response.refusal.*` | map refusal → `Text` + warning `model_refusal` | `Other` unless explicit incomplete/filter reason | OK + warning (ambiguity) |
| Error returned | `status=failed` with `error:{code,message}` | **No ProviderResponse** | n/a | `Err(ProviderError::Protocol/Status)` |
| Empty output | `status=completed`, `output=[]` | `content=[]` | `Stop` or `Other` (recommend `Other` + warning) | OK + warning `empty_output` |
| Missing usage | `usage:null` (seen in cancelled/streaming objects) | `usage=Usage::default()` | per status mapping | OK + warning `usage_missing` |
| Unexpected status value | status not in documented set | none | n/a | `Err(ProviderError::Protocol("unknown_status"))` |

## Deterministic translation algorithms

This section consolidates the essential pseudocode algorithms required by your deliverables.

### encode_request(req) pseudocode (canonical → OpenAI)

```text
function encode_request(req):
  validate provider_hint: if Some != Openai -> Protocol error
  validate model_id non-empty
  validate metadata constraints (<=16, key<=64, value<=512) else error
  validate stop empty else Protocol error (unsupported)
  validate tool_choice Specific -> Protocol error unless canonical changed

  # Response format -> text.format
  text_format = map_response_format(req.response_format)
  if req.response_format == JsonObject:
    if not any_text_contains_substring(req.messages, "JSON"):
      return Protocol error ("JSON mode requires 'JSON' in context per OpenAI")

  # Tools
  tools_payload = map_tools(req.tools)  # compute strict mode only if schema complies
  tool_choice_payload = map_tool_choice(req.tool_choice)

  # Messages -> input items
  input_items = []
  for message in req.messages:
    normalize_message_into_input_items(message, input_items)
    # - Text parts become message item with content[] input_text parts
    # - ToolCall parts become function_call items
    # - ToolResult parts become function_call_output items

  if input_items empty:
    return Protocol error ("empty input")

  payload = {
    "model": req.model.model_id,
    "input": input_items,
    "text": { "format": text_format },
    "tools": tools_payload,
    "tool_choice": tool_choice_payload,
    "temperature": req.temperature?,
    "top_p": req.top_p?,
    "max_output_tokens": req.max_output_tokens?,
    "metadata": req.metadata?
  }
  return Ok(payload)
```

Grounding: request parameters and semantics come from the Responses API reference and the structured outputs + function calling guides.

### strict-compatible schema check pseudocode (for function strict mode)

```text
function is_strict_compatible(schema):
  # schema must be an object-like JSON schema (best-effort)
  if schema.type == "object":
     if schema.additionalProperties != false: return false
     props = schema.properties keys
     req = schema.required array
     if set(req) != set(props): return false
     # recurse into nested object schemas in properties
     for each prop_schema in schema.properties:
        if prop_schema.type includes "object": 
           if not is_strict_compatible(prop_schema): return false
  if schema contains anyOf/oneOf/allOf:
     # allowed by JSON schema, but strict mode support unclear in allowed sources
     # deterministic choice: return false to avoid potential provider rejection
     return false
  return true
```

Grounding: strict-mode requirements are explicitly described.

### decode_response(payload) pseudocode

(See the earlier detailed decode pseudocode; it is normative.)

Grounding: OpenAI response object `status`, `output`, and function calling output item structure.

## Stage 14 scope lock (canonical-complete, non-streaming)

- Stage 14 implementation is intentionally non-streaming for v0.
- “Full OpenAI Responses API surface” in this stage means:
  - full non-streaming coverage for all behaviors representable by current canonical types, and
  - deterministic `ProviderError::Protocol` for OpenAI-only item types or payload shapes that
    cannot be represented without changing canonical public interfaces.

## Final completion criterion

“The OpenAI adapter is complete when all canonical semantics are preserved without transport leakage and the translator contract is fully satisfied.”
