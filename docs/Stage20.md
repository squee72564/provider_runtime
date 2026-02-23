# Stage 20 - Providers Module Wiring

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

Internal Responsibilities:
- Module visibility boundaries.
- Keep translator modules crate-private.

Unit Tests:
- `test_providers_module_exports_compile`

Acceptance Criteria:
- Runtime can consume adapters.
- Translator internals not public API.
- `cargo check --lib` passes.

Depends On:
- Stage 15
- Stage 17
- Stage 19
