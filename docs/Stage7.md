# Stage 7 - Pricing Engine (Optional, Warning-Based)

Goal:
Implement cost estimation from canonical usage + configurable pricing table.

Files:
- `src/pricing/mod.rs` (not in repo yet)

Public API Surface:
- `struct PricingTable`
- `struct PriceRule`
- `fn estimate_cost(...) -> (Option<CostBreakdown>, Vec<RuntimeWarning>)`

Internal Responsibilities:
- Match provider/model to price rule.
- Compute input/output costs when available.
- Return warning when pricing or required usage is missing/partial.

Unit Tests:
- `test_estimate_cost_known_model`
- `test_missing_price_returns_warning_not_error`
- `test_partial_usage_handles_optional_fields`

Acceptance Criteria:
- Cost never blocks successful provider responses.
- No provider adapter dependency on pricing internals.
- `cargo check --lib` passes.

Depends On:
- Stage 1
- Stage 2
