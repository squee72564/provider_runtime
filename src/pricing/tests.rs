use super::*;

fn single_rule_table(rule: PriceRule) -> PricingTable {
    PricingTable::new(vec![rule])
}

#[test]
fn test_estimate_cost_known_model() {
    let table = single_rule_table(PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5-mini".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
        reasoning_cost_per_token: Some(0.03),
    });
    let usage = Usage {
        input_tokens: Some(10),
        output_tokens: Some(20),
        reasoning_tokens: Some(5),
        cached_input_tokens: Some(7),
        total_tokens: Some(35),
    };

    let (cost, warnings) = estimate_cost(&ProviderId::Openai, "gpt-5-mini", &usage, &table);

    assert!(warnings.is_empty());
    let cost = cost.expect("cost should be estimated");
    assert_eq!(cost.currency, "USD");
    assert_eq!(cost.input_cost, 0.1);
    assert_eq!(cost.output_cost, 0.4);
    assert_eq!(cost.reasoning_cost, Some(0.15));
    assert_eq!(cost.total_cost, 0.65);
    assert_eq!(cost.pricing_source, PricingSource::Configured);
}

#[test]
fn test_missing_price_returns_warning_not_error() {
    let table = PricingTable::new(Vec::new());
    let usage = Usage {
        input_tokens: Some(1),
        output_tokens: Some(2),
        reasoning_tokens: None,
        cached_input_tokens: None,
        total_tokens: None,
    };

    let (cost, warnings) =
        estimate_cost(&ProviderId::Openrouter, "openrouter/test", &usage, &table);

    assert!(cost.is_none());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "pricing.missing_rule");
}

#[test]
fn test_partial_usage_handles_optional_fields() {
    let table = single_rule_table(PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-5-mini".to_string(),
        input_cost_per_token: 0.01,
        output_cost_per_token: 0.02,
        reasoning_cost_per_token: Some(0.03),
    });
    let usage = Usage {
        input_tokens: Some(10),
        output_tokens: None,
        reasoning_tokens: None,
        cached_input_tokens: None,
        total_tokens: None,
    };

    let (cost, warnings) = estimate_cost(&ProviderId::Openai, "gpt-5-mini", &usage, &table);

    let cost = cost.expect("cost should still be produced");
    assert_eq!(cost.input_cost, 0.1);
    assert_eq!(cost.output_cost, 0.0);
    assert_eq!(cost.reasoning_cost, None);
    assert_eq!(cost.total_cost, 0.1);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "pricing.partial_usage");
}

#[test]
fn test_rule_resolution_prefers_exact_over_wildcard() {
    let table = PricingTable::new(vec![
        PriceRule {
            provider: ProviderId::Openai,
            model_pattern: "gpt-*".to_string(),
            input_cost_per_token: 1.0,
            output_cost_per_token: 1.0,
            reasoning_cost_per_token: None,
        },
        PriceRule {
            provider: ProviderId::Openai,
            model_pattern: "gpt-5-mini".to_string(),
            input_cost_per_token: 2.0,
            output_cost_per_token: 2.0,
            reasoning_cost_per_token: None,
        },
    ]);

    let rule = table
        .find_rule(&ProviderId::Openai, "gpt-5-mini")
        .expect("rule should exist");

    assert_eq!(rule.model_pattern, "gpt-5-mini");
    assert_eq!(rule.input_cost_per_token, 2.0);
}

#[test]
fn test_rule_resolution_uses_longest_wildcard_prefix() {
    let table = PricingTable::new(vec![
        PriceRule {
            provider: ProviderId::Openai,
            model_pattern: "*".to_string(),
            input_cost_per_token: 1.0,
            output_cost_per_token: 1.0,
            reasoning_cost_per_token: None,
        },
        PriceRule {
            provider: ProviderId::Openai,
            model_pattern: "gpt-*".to_string(),
            input_cost_per_token: 2.0,
            output_cost_per_token: 2.0,
            reasoning_cost_per_token: None,
        },
        PriceRule {
            provider: ProviderId::Openai,
            model_pattern: "gpt-5-*".to_string(),
            input_cost_per_token: 3.0,
            output_cost_per_token: 3.0,
            reasoning_cost_per_token: None,
        },
    ]);

    let rule = table
        .find_rule(&ProviderId::Openai, "gpt-5-mini")
        .expect("rule should exist");

    assert_eq!(rule.model_pattern, "gpt-5-*");
    assert_eq!(rule.input_cost_per_token, 3.0);
}

#[test]
fn test_reasoning_tokens_without_reasoning_rate_warns() {
    let table = single_rule_table(PriceRule {
        provider: ProviderId::Anthropic,
        model_pattern: "claude-*".to_string(),
        input_cost_per_token: 0.1,
        output_cost_per_token: 0.2,
        reasoning_cost_per_token: None,
    });
    let usage = Usage {
        input_tokens: Some(1),
        output_tokens: Some(2),
        reasoning_tokens: Some(3),
        cached_input_tokens: None,
        total_tokens: None,
    };

    let (cost, warnings) =
        estimate_cost(&ProviderId::Anthropic, "claude-3-7-sonnet", &usage, &table);

    let cost = cost.expect("cost should still be produced");
    assert_eq!(cost.reasoning_cost, None);
    assert_eq!(cost.total_cost, 0.5);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "pricing.partial_reasoning_rate");
}

#[test]
fn test_invalid_rate_returns_warning_and_none_cost() {
    let table = single_rule_table(PriceRule {
        provider: ProviderId::Openrouter,
        model_pattern: "openrouter/*".to_string(),
        input_cost_per_token: 0.1,
        output_cost_per_token: -0.2,
        reasoning_cost_per_token: None,
    });
    let usage = Usage {
        input_tokens: Some(2),
        output_tokens: Some(3),
        reasoning_tokens: None,
        cached_input_tokens: None,
        total_tokens: None,
    };

    let (cost, warnings) =
        estimate_cost(&ProviderId::Openrouter, "openrouter/test", &usage, &table);

    assert!(cost.is_none());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "pricing.invalid_rule");
}

#[test]
fn test_no_usage_tokens_returns_none_with_warning() {
    let table = single_rule_table(PriceRule {
        provider: ProviderId::Openai,
        model_pattern: "gpt-*".to_string(),
        input_cost_per_token: 0.1,
        output_cost_per_token: 0.2,
        reasoning_cost_per_token: Some(0.3),
    });
    let usage = Usage::default();

    let (cost, warnings) = estimate_cost(&ProviderId::Openai, "gpt-5-mini", &usage, &table);

    assert!(cost.is_none());
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, "pricing.missing_usage");
}
