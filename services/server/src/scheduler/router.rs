use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::models::{AccountRouteState, CliLease, RouteMode, UpstreamAccount};

pub fn select_dual_candidates<'a>(
    principal_id: &str,
    model: &str,
    accounts: &'a [UpstreamAccount],
) -> Vec<&'a UpstreamAccount> {
    let mut ranked = accounts
        .iter()
        .map(|account| {
            (
                account,
                rendezvous_score(principal_id, model, &account.id, "a"),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = ranked
        .into_iter()
        .take(2)
        .map(|(account, _)| account)
        .collect::<Vec<_>>();
    if out.len() < 2 {
        return out;
    }

    let second = accounts
        .iter()
        .filter(|account| account.id != out[0].id)
        .map(|account| {
            (
                account,
                rendezvous_score(principal_id, model, &account.id, "b"),
            )
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(account, _)| account);
    if let Some(second) = second {
        out[1] = second;
    }
    out
}

pub fn score_candidate(
    account: &UpstreamAccount,
    route_state: &AccountRouteState,
    existing_lease: Option<&CliLease>,
) -> f64 {
    let cache_affinity = if existing_lease.is_some_and(|lease| {
        lease.account_id == account.id && account.models.contains(&lease.model)
    }) {
        1.0
    } else {
        0.35
    };

    let headroom = if account.signals.capacity == 0 {
        1.0
    } else {
        1.0 - (account.signals.inflight as f64 / account.signals.capacity as f64)
    }
    .clamp(0.0, 1.0);

    let warp_bonus = match route_state.route_mode {
        RouteMode::Direct => 0.08,
        RouteMode::Warp => 0.04,
    };

    0.35 * cache_affinity
        + 0.20 * account.signals.effective_quota_headroom().clamp(0.0, 1.0)
        + 0.15 * account.signals.health_score.clamp(0.0, 1.0)
        + 0.10 * account.signals.egress_stability.clamp(0.0, 1.0)
        + 0.10 * account.signals.fairness_bias.clamp(0.0, 1.0)
        + 0.10 * headroom
        + warp_bonus
}

pub fn should_reuse_lease(
    lease: &CliLease,
    account: &UpstreamAccount,
    route_state: &AccountRouteState,
) -> bool {
    if lease.account_id != account.id || !account.models.contains(&lease.model) {
        return false;
    }
    if route_state
        .cooldown_until
        .is_some_and(|until| until > Utc::now())
    {
        return false;
    }
    true
}

fn rendezvous_score(principal_id: &str, model: &str, account_id: &str, salt: &str) -> f64 {
    let digest = Sha256::digest(format!("{principal_id}:{model}:{account_id}:{salt}").as_bytes());
    let value = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    value as f64 / u64::MAX as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn account(id: &str, health: f64) -> UpstreamAccount {
        UpstreamAccount {
            id: id.to_string(),
            tenant_id: Uuid::nil(),
            label: id.to_string(),
            models: vec!["gpt-5.4".to_string()],
            current_mode: RouteMode::Direct,
            signals: crate::models::SchedulingSignals {
                quota_headroom: 0.9,
                quota_headroom_5h: 0.9,
                quota_headroom_7d: 0.9,
                health_score: health,
                egress_stability: 0.8,
                fairness_bias: 0.7,
                inflight: 0,
                capacity: 4,
            },
            created_at: Utc::now(),
        }
    }

    fn route_state(account_id: &str) -> AccountRouteState {
        AccountRouteState {
            account_id: account_id.to_string(),
            route_mode: RouteMode::Direct,
            direct_cf_streak: 0,
            warp_cf_streak: 0,
            cooldown_level: 0,
            cooldown_until: None,
            cooldown_reason: None,
            warp_entered_at: None,
            last_cf_at: None,
            success_streak: 0,
            last_success_at: None,
        }
    }

    #[test]
    fn dual_candidates_are_bounded_to_two() {
        let accounts = vec![account("a", 0.8), account("b", 0.9), account("c", 0.7)];
        let actual = select_dual_candidates("principal", "gpt-5.4", &accounts);
        assert!(actual.len() <= 2);
        assert!(!actual.is_empty());
    }

    #[test]
    fn score_candidate_favors_better_health() {
        let existing = None;
        let weaker = score_candidate(&account("a", 0.4), &route_state("a"), existing);
        let stronger = score_candidate(&account("b", 0.9), &route_state("b"), existing);
        assert!(stronger > weaker);
    }
}
