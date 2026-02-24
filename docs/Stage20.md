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
- Keep translator contract internal to provider layer and non-reexported.
- Enforce separation:
  - adapter modules (`openai`, `anthropic`, `openrouter`) own orchestration
  - translator modules (`*_translate`) own canonical<->wire mapping

Unit Tests:
- `test_providers_module_exports_compile`
- `test_translator_modules_not_public`

Acceptance Criteria:
- Runtime can consume adapters.
- Translator internals not public API.
- Stage module paths for translator stages (14/16/18) align to `*_translate` modules.
- `cargo check --lib` passes.

Depends On:
- Stage 15
- Stage 17
- Stage 19
