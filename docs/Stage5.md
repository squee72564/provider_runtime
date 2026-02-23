# Stage 5 - HTTP Transport Abstraction

Goal:
Implement shared async HTTP utility used by adapters.

Files:
- [src/transport/http.rs](src/transport/http.rs)

Public API Surface:
- `struct HttpTransport`
- `struct RetryPolicy`
- `impl HttpTransport` methods for JSON request/response

Internal Responsibilities:
- Timeout and retry policy.
- Header/auth injection hooks from `AdapterContext`.
- Structured transport-level error mapping.

Unit Tests:
- `test_http_transport_maps_status_errors`
- `test_retry_policy_respects_max_attempts`

Acceptance Criteria:
- Transport reusable by all adapters.
- No provider-specific parsing logic.
- `cargo check --lib` passes.

Depends On:
- Stage 2
- Stage 4
