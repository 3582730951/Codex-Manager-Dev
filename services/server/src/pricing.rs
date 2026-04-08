use crate::models::RequestLogUsage;

#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_million_usd: f64,
    pub cached_input_per_million_usd: f64,
    pub output_per_million_usd: f64,
}

pub fn pricing_for_model(model: &str) -> Option<ModelPricing> {
    let normalized = model.trim().to_ascii_lowercase();

    for (prefix, pricing) in [
        (
            "gpt-5.4-mini",
            ModelPricing {
                input_per_million_usd: 0.75,
                cached_input_per_million_usd: 0.075,
                output_per_million_usd: 4.50,
            },
        ),
        (
            "gpt-5.4-nano",
            ModelPricing {
                input_per_million_usd: 0.20,
                cached_input_per_million_usd: 0.02,
                output_per_million_usd: 1.25,
            },
        ),
        (
            "gpt-5.4",
            ModelPricing {
                input_per_million_usd: 2.50,
                cached_input_per_million_usd: 0.25,
                output_per_million_usd: 15.00,
            },
        ),
        (
            "gpt-5.3-codex",
            ModelPricing {
                input_per_million_usd: 1.75,
                cached_input_per_million_usd: 0.175,
                output_per_million_usd: 14.00,
            },
        ),
        (
            "gpt-5.2",
            ModelPricing {
                input_per_million_usd: 1.75,
                cached_input_per_million_usd: 0.175,
                output_per_million_usd: 14.00,
            },
        ),
        (
            "gpt-5",
            ModelPricing {
                input_per_million_usd: 1.25,
                cached_input_per_million_usd: 0.125,
                output_per_million_usd: 10.00,
            },
        ),
        (
            "gpt-4.1-mini",
            ModelPricing {
                input_per_million_usd: 0.40,
                cached_input_per_million_usd: 0.10,
                output_per_million_usd: 1.60,
            },
        ),
        (
            "gpt-4.1-nano",
            ModelPricing {
                input_per_million_usd: 0.10,
                cached_input_per_million_usd: 0.025,
                output_per_million_usd: 0.40,
            },
        ),
        (
            "gpt-4.1",
            ModelPricing {
                input_per_million_usd: 2.00,
                cached_input_per_million_usd: 0.50,
                output_per_million_usd: 8.00,
            },
        ),
    ] {
        if normalized == prefix || normalized.starts_with(&format!("{prefix}-")) {
            return Some(pricing);
        }
    }

    None
}

pub fn estimate_cost_usd(model: &str, usage: &RequestLogUsage) -> Option<f64> {
    let pricing = pricing_for_model(model)?;
    let cached_input = usage.cached_input_tokens.min(usage.input_tokens);
    let live_input = usage.input_tokens.saturating_sub(cached_input);

    let total = (live_input as f64 / 1_000_000.0) * pricing.input_per_million_usd
        + (cached_input as f64 / 1_000_000.0) * pricing.cached_input_per_million_usd
        + (usage.output_tokens as f64 / 1_000_000.0) * pricing.output_per_million_usd;

    Some((total * 1_000_000.0).round() / 1_000_000.0)
}
