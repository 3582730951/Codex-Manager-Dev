#[derive(Debug, Clone, Copy)]
pub struct WarmupDecision {
    pub should_warm: bool,
    pub expected_saving: f64,
}

pub fn evaluate_prefix_warmup(
    predicted_turns_remaining: u32,
    reusable_prefix_tokens: u64,
    cache_discount_ratio: f64,
    warmup_tokens: u64,
    congested: bool,
) -> WarmupDecision {
    if congested || predicted_turns_remaining <= 1 || reusable_prefix_tokens < 1024 {
        return WarmupDecision {
            should_warm: false,
            expected_saving: 0.0,
        };
    }
    let expected_saving = ((predicted_turns_remaining - 1) as f64
        * reusable_prefix_tokens as f64
        * cache_discount_ratio)
        - warmup_tokens as f64;

    WarmupDecision {
        should_warm: expected_saving > 0.0,
        expected_saving,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_requires_positive_roi() {
        let decision = evaluate_prefix_warmup(4, 4096, 0.75, 1024, false);
        assert!(decision.should_warm);
        assert!(decision.expected_saving > 0.0);
    }

    #[test]
    fn warmup_is_disabled_under_congestion() {
        let decision = evaluate_prefix_warmup(4, 4096, 0.75, 512, true);
        assert!(!decision.should_warm);
    }
}
