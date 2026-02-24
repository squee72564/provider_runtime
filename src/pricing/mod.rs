use crate::core::types::{CostBreakdown, PricingSource, ProviderId, RuntimeWarning, Usage};

#[derive(Debug, Clone, PartialEq)]
pub struct PriceRule {
    pub provider: ProviderId,
    pub model_pattern: String,
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub reasoning_cost_per_token: Option<f64>,
}

impl PriceRule {
    fn has_valid_rates(&self) -> bool {
        is_valid_rate(self.input_cost_per_token)
            && is_valid_rate(self.output_cost_per_token)
            && self.reasoning_cost_per_token.is_none_or(is_valid_rate)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PricingTable {
    pub rules: Vec<PriceRule>,
}

impl PricingTable {
    pub fn new(rules: Vec<PriceRule>) -> Self {
        Self { rules }
    }

    pub fn find_rule(&self, provider: &ProviderId, model: &str) -> Option<&PriceRule> {
        let mut best_index: Option<usize> = None;
        let mut best_score: Option<RuleMatchScore> = None;

        for (index, rule) in self.rules.iter().enumerate() {
            if rule.provider != *provider {
                continue;
            }

            let Some(score) = match_pattern(&rule.model_pattern, model) else {
                continue;
            };

            let should_replace = match best_score {
                Some(current) => score > current,
                None => true,
            };

            if should_replace {
                best_index = Some(index);
                best_score = Some(score);
            }
        }

        best_index.map(|index| &self.rules[index])
    }
}

pub fn estimate_cost(
    provider: &ProviderId,
    model: &str,
    usage: &Usage,
    table: &PricingTable,
) -> (Option<CostBreakdown>, Vec<RuntimeWarning>) {
    let mut warnings = Vec::new();

    let Some(rule) = table.find_rule(provider, model) else {
        warnings.push(RuntimeWarning {
            code: "pricing.missing_rule".to_string(),
            message: format!("no pricing rule configured for provider={provider:?}, model={model}"),
        });
        return (None, warnings);
    };

    if !rule.has_valid_rates() {
        warnings.push(RuntimeWarning {
            code: "pricing.invalid_rule".to_string(),
            message: format!(
                "invalid pricing rule for provider={provider:?}, model_pattern={}",
                rule.model_pattern
            ),
        });
        return (None, warnings);
    }

    let has_any_usage = usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.reasoning_tokens.is_some();
    if !has_any_usage {
        warnings.push(RuntimeWarning {
            code: "pricing.missing_usage".to_string(),
            message: format!("usage tokens missing for provider={provider:?}, model={model}"),
        });
        return (None, warnings);
    }

    if usage.input_tokens.is_none() || usage.output_tokens.is_none() {
        warnings.push(RuntimeWarning {
            code: "pricing.partial_usage".to_string(),
            message: format!(
                "partial usage for provider={provider:?}, model={model}; missing input or output tokens"
            ),
        });
    }

    let input_cost = usage.input_tokens.unwrap_or(0) as f64 * rule.input_cost_per_token;
    let output_cost = usage.output_tokens.unwrap_or(0) as f64 * rule.output_cost_per_token;

    let reasoning_cost = match (usage.reasoning_tokens, rule.reasoning_cost_per_token) {
        (Some(tokens), Some(rate)) => Some(tokens as f64 * rate),
        (Some(_), None) => {
            warnings.push(RuntimeWarning {
                code: "pricing.partial_reasoning_rate".to_string(),
                message: format!(
                    "reasoning tokens provided but no reasoning rate configured for provider={provider:?}, model={model}"
                ),
            });
            None
        }
        (None, _) => None,
    };

    let total_cost = input_cost + output_cost + reasoning_cost.unwrap_or(0.0);

    (
        Some(CostBreakdown {
            currency: "USD".to_string(),
            input_cost,
            output_cost,
            reasoning_cost,
            total_cost,
            pricing_source: PricingSource::Configured,
        }),
        warnings,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RuleMatchScore {
    exact: bool,
    prefix_len: usize,
}

fn match_pattern(pattern: &str, model: &str) -> Option<RuleMatchScore> {
    if pattern == model {
        return Some(RuleMatchScore {
            exact: true,
            prefix_len: pattern.len(),
        });
    }

    if pattern == "*" {
        return Some(RuleMatchScore {
            exact: false,
            prefix_len: 0,
        });
    }

    let prefix = pattern.strip_suffix('*')?;
    if model.starts_with(prefix) {
        return Some(RuleMatchScore {
            exact: false,
            prefix_len: prefix.len(),
        });
    }

    None
}

fn is_valid_rate(rate: f64) -> bool {
    rate.is_finite() && rate >= 0.0
}

#[cfg(test)]
mod tests;
