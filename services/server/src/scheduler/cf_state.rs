use chrono::{DateTime, Duration, Utc};

use crate::models::{AccountRouteState, RouteMode};

const COOLDOWN_STEPS_SECONDS: [i64; 8] = [5, 60, 300, 600, 900, 1800, 3600, 43_200];
const WARP_EJECTION_HOURS: i64 = 72;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfOutcome {
    pub switched_to_warp: bool,
    pub failover_required: bool,
}

pub fn register_cf_hit(
    state: &mut AccountRouteState,
    mode: RouteMode,
    now: DateTime<Utc>,
) -> CfOutcome {
    state.last_cf_at = Some(now);
    state.success_streak = 0;

    match mode {
        RouteMode::Direct => {
            state.direct_cf_streak += 1;
            if state.direct_cf_streak >= 3 {
                state.route_mode = RouteMode::Warp;
                state.direct_cf_streak = 0;
                state.warp_entered_at = Some(now);
                return CfOutcome {
                    switched_to_warp: true,
                    failover_required: false,
                };
            }
            CfOutcome {
                switched_to_warp: false,
                failover_required: false,
            }
        }
        RouteMode::Warp => {
            state.warp_cf_streak += 1;
            state.cooldown_level = (state.cooldown_level + 1).min(COOLDOWN_STEPS_SECONDS.len() - 1);
            state.cooldown_until =
                Some(now + Duration::seconds(COOLDOWN_STEPS_SECONDS[state.cooldown_level]));
            CfOutcome {
                switched_to_warp: false,
                failover_required: true,
            }
        }
    }
}

pub fn register_success(state: &mut AccountRouteState, now: DateTime<Utc>) {
    state.success_streak += 1;
    state.last_success_at = Some(now);
    if state.success_streak >= 20 && state.cooldown_level > 0 {
        state.cooldown_level -= 1;
    }
    if let Some(warp_entered_at) = state.warp_entered_at {
        if now - warp_entered_at >= Duration::hours(WARP_EJECTION_HOURS) {
            state.route_mode = RouteMode::Direct;
            state.warp_entered_at = None;
            state.warp_cf_streak = 0;
            state.direct_cf_streak = 0;
        }
    }
}

pub fn is_in_cooldown(state: &AccountRouteState, now: DateTime<Utc>) -> bool {
    state.cooldown_until.is_some_and(|until| until > now)
}

pub fn reconcile_route_mode(state: &mut AccountRouteState, now: DateTime<Utc>) {
    if let Some(warp_entered_at) = state.warp_entered_at {
        if now - warp_entered_at >= Duration::hours(WARP_EJECTION_HOURS) {
            state.route_mode = RouteMode::Direct;
            state.warp_entered_at = None;
            state.warp_cf_streak = 0;
            state.direct_cf_streak = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_state(mode: RouteMode) -> AccountRouteState {
        AccountRouteState {
            account_id: "acc-1".to_string(),
            route_mode: mode,
            direct_cf_streak: 0,
            warp_cf_streak: 0,
            cooldown_level: 0,
            cooldown_until: None,
            warp_entered_at: None,
            last_cf_at: None,
            success_streak: 0,
            last_success_at: None,
        }
    }

    #[test]
    fn direct_cf_enters_warp_after_three_hits() {
        let now = Utc::now();
        let mut state = base_state(RouteMode::Direct);
        for _ in 0..2 {
            let outcome = register_cf_hit(&mut state, RouteMode::Direct, now);
            assert!(!outcome.switched_to_warp);
        }
        let outcome = register_cf_hit(&mut state, RouteMode::Direct, now);
        assert!(outcome.switched_to_warp);
        assert_eq!(state.route_mode, RouteMode::Warp);
    }

    #[test]
    fn warp_cf_escalates_cooldown_and_requires_failover() {
        let now = Utc::now();
        let mut state = base_state(RouteMode::Warp);
        let outcome = register_cf_hit(&mut state, RouteMode::Warp, now);
        assert!(outcome.failover_required);
        assert_eq!(state.cooldown_level, 1);
        assert!(state.cooldown_until.is_some());
    }

    #[test]
    fn warp_is_ejected_after_seventy_two_hours() {
        let mut state = base_state(RouteMode::Warp);
        state.warp_entered_at = Some(Utc::now() - Duration::hours(73));
        reconcile_route_mode(&mut state, Utc::now());
        assert_eq!(state.route_mode, RouteMode::Direct);
    }
}
