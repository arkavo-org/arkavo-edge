//! Three-plane router separation.
//!
//! ```text
//! Availability changes what is *possible*  -> feasibility plane
//! Quality changes what is *advisable*       -> collapse plane (adequacy v1)
//! Budget policy changes what is *allowed*   -> spend plane (arkavo-budget)
//! ```
//!
//! The planes are kept strictly separate so a feasibility/availability failure
//! can never silently authorize cloud spend. The flow this module encodes:
//!
//! 1. Check local feasibility.
//! 2. If local can run, run local.
//! 3. If local visibly collapses, mark it as collapse.
//! 4. If local completes, show the answer.
//! 5. Offer a cloud upgrade when: the user asks, a collapse occurred, the task
//!    class is high-risk, and the user policy allows asking.
//! 6. Before any cloud call, check budget.
//! 7. If budget is exhausted, stay local.
//!
//! The key distinction: a collapse can trigger an upgrade *offer*; it does not
//! trigger automatic spend by default.

pub mod collapse;
pub mod feasibility;
pub mod feasibility_baseline;

pub use collapse::{AnswerObservation, CollapseSignal, CollapseVerdict, detect as detect_collapse};
pub use feasibility::{FeasibilityVerdict, RuntimeStats, assess as assess_feasibility};
pub use feasibility_baseline::{FeasibilityBaseline, FeasibilityBaselineSnapshot};

use arkavo_budget::TokenCost;
use arkavo_budget::cloud_policy::{
    CloudPolicy, CloudSpendDecision, CloudSpendReason, CloudSpendRequest, SpendCaps,
    authorize_cloud_spend,
};

/// What to do with the request given only the feasibility plane's verdict.
///
/// [`LocalPlan::Unavailable`] is **not** a spend authorization: the caller must
/// ask the user, queue, retry locally, or return a partial answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalPlan {
    /// Run the request on the local model.
    RunLocal,
    /// Run locally, but tell the user it will be slow (busy/throttled).
    RunLocalSlow,
    /// Reshape the request first (chunk or shrink context), then run local.
    Reshape(FeasibilityVerdict),
    /// Local cannot run right now. Availability failure — never silent spend.
    Unavailable,
}

/// Map the feasibility verdict to a local execution plan. The feasibility plane
/// alone decides this; no cloud action is reachable from here.
pub fn plan_local(stats: &RuntimeStats) -> LocalPlan {
    match assess_feasibility(stats) {
        FeasibilityVerdict::LocalCanRunNow => LocalPlan::RunLocal,
        FeasibilityVerdict::LocalCanRunSlowly => LocalPlan::RunLocalSlow,
        reshape @ (FeasibilityVerdict::LocalNeedsChunking
        | FeasibilityVerdict::LocalNeedsSmallerContext) => LocalPlan::Reshape(reshape),
        FeasibilityVerdict::LocalCannotRun => LocalPlan::Unavailable,
    }
}

/// Conditions, independent of feasibility, under which a cloud upgrade may be
/// *offered* after a local answer.
#[derive(Debug, Clone, Copy, Default)]
pub struct UpgradeContext {
    /// The user explicitly asked for a cloud / higher-quality answer.
    pub user_requested: bool,
    /// The task class is high-risk (e.g. security-sensitive, irreversible).
    pub high_risk_task: bool,
}

/// Whether — and why — to offer a cloud upgrade. Presenting an offer never
/// spends; the offer must still pass the spend plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeOffer {
    /// Do not offer cloud; keep the local answer.
    None,
    /// Offer the user a cloud upgrade for the stated reason.
    Offer(CloudSpendReason),
}

/// Decide whether to offer a cloud upgrade after a completed local answer.
///
/// Only quality (collapse) and user/task context drive an offer — never a
/// feasibility signal. `LocalOnly` policy never offers.
pub fn upgrade_offer(
    policy: CloudPolicy,
    collapse: &CollapseVerdict,
    ctx: UpgradeContext,
) -> UpgradeOffer {
    if policy == CloudPolicy::LocalOnly {
        return UpgradeOffer::None;
    }
    if ctx.user_requested {
        return UpgradeOffer::Offer(CloudSpendReason::UserRequested);
    }
    if collapse.collapsed() {
        return UpgradeOffer::Offer(CloudSpendReason::LocalCollapsed);
    }
    if ctx.high_risk_task {
        return UpgradeOffer::Offer(CloudSpendReason::HighRiskTask);
    }
    UpgradeOffer::None
}

/// Authorize cloud spend for an offered upgrade.
///
/// This is the ONLY path from a collapse/offer to actual spend, and it
/// delegates the allow/deny decision to the budget (spend) plane. Feasibility
/// signals never reach this function.
pub fn authorize_upgrade(
    policy: CloudPolicy,
    reason: CloudSpendReason,
    projected_cost: TokenCost,
    caps: SpendCaps,
    user_confirmed: bool,
) -> CloudSpendDecision {
    let request = CloudSpendRequest {
        reason,
        projected_cost,
        user_confirmed,
    };
    authorize_cloud_spend(policy, &request, caps)
}

/// Augment a re-route exclusion set per the plane separation.
///
/// A retry triggered by a feasibility failure (timeout/OOM) or a quality
/// collapse may not silently cross into paid cloud spend. Unless the cloud
/// policy authorizes spending without asking
/// ([`CloudPolicy::permits_silent_cloud`]), every cloud arm is added to the
/// `excluded` set so re-selection stays local. This is the single place the
/// "availability/quality may not silently spend" rule is enforced in the
/// routing loop, and it is pure so it can be unit-tested without a model.
pub fn augment_exclusions_for_policy(
    mut excluded: Vec<String>,
    policy: CloudPolicy,
) -> Vec<String> {
    if !policy.permits_silent_cloud() {
        for model in crate::decision::ModelChoice::ALL_CLOUD {
            let name = model.name().to_string();
            if !excluded.iter().any(|e| e == &name) {
                excluded.push(name);
            }
        }
    }
    excluded
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn healthy_stats() -> RuntimeStats {
        RuntimeStats {
            prompt_tokens: 500,
            n_ctx: 8_192,
            local_model_available: true,
            kv_cache_slots_free: 4,
            tokens_per_sec: Some(40.0),
            context_overflow: false,
        }
    }

    #[spec("ROUTER-021")]
    #[test]
    fn healthy_runtime_runs_local() {
        assert_eq!(plan_local(&healthy_stats()), LocalPlan::RunLocal);
    }

    #[test]
    fn slow_runtime_runs_local_slow_not_cloud() {
        let stats = RuntimeStats {
            tokens_per_sec: Some(3.0),
            ..healthy_stats()
        };
        // A slow laptop stays local. There is no cloud variant in LocalPlan.
        assert_eq!(plan_local(&stats), LocalPlan::RunLocalSlow);
    }

    #[test]
    fn unavailable_local_is_not_a_spend_event() {
        let stats = RuntimeStats {
            local_model_available: false,
            ..healthy_stats()
        };
        assert_eq!(plan_local(&stats), LocalPlan::Unavailable);
    }

    // Corrected escalation behavior: a feasibility degradation produces no
    // upgrade offer on its own. Only collapse / user-ask / high-risk do.
    #[test]
    fn slow_local_alone_does_not_offer_cloud() {
        let offer = upgrade_offer(
            CloudPolicy::AskBeforeCloud,
            &CollapseVerdict::NoVisibleCollapse,
            UpgradeContext::default(),
        );
        assert_eq!(offer, UpgradeOffer::None);
    }

    #[spec("ROUTER-022")]
    #[test]
    fn collapse_offers_cloud_upgrade() {
        let offer = upgrade_offer(
            CloudPolicy::AskBeforeCloud,
            &CollapseVerdict::Collapsed(CollapseSignal::RepetitionLoop),
            UpgradeContext::default(),
        );
        assert_eq!(offer, UpgradeOffer::Offer(CloudSpendReason::LocalCollapsed));
    }

    #[test]
    fn user_request_offers_cloud_upgrade() {
        let offer = upgrade_offer(
            CloudPolicy::AskBeforeCloud,
            &CollapseVerdict::NoVisibleCollapse,
            UpgradeContext {
                user_requested: true,
                ..Default::default()
            },
        );
        assert_eq!(offer, UpgradeOffer::Offer(CloudSpendReason::UserRequested));
    }

    #[spec("ROUTER-022")]
    #[test]
    fn local_only_never_offers_even_on_collapse() {
        let offer = upgrade_offer(
            CloudPolicy::LocalOnly,
            &CollapseVerdict::Collapsed(CollapseSignal::EmptyOutput),
            UpgradeContext {
                user_requested: true,
                high_risk_task: true,
            },
        );
        assert_eq!(offer, UpgradeOffer::None);
    }

    // End-to-end of the corrected logic: collapse -> offer -> the spend plane
    // still gates the actual call. Under the default policy that means asking.
    #[test]
    fn collapse_offer_then_spend_plane_asks_before_spending() {
        let policy = CloudPolicy::AskBeforeCloud;
        let collapse = CollapseVerdict::Collapsed(CollapseSignal::TruncatedOutput);
        let UpgradeOffer::Offer(reason) =
            upgrade_offer(policy, &collapse, UpgradeContext::default())
        else {
            panic!("collapse should produce an offer");
        };

        let caps = SpendCaps {
            remaining_cap: TokenCost::from_dollars(10.0),
            per_request_max: None,
        };
        // Not yet confirmed → the spend plane asks, it does not auto-spend.
        let decision = authorize_upgrade(policy, reason, TokenCost::from_dollars(0.2), caps, false);
        assert!(matches!(
            decision,
            CloudSpendDecision::NeedsUserConfirmation { .. }
        ));

        // After the user confirms, the same request is authorized.
        let confirmed = authorize_upgrade(policy, reason, TokenCost::from_dollars(0.2), caps, true);
        assert!(confirmed.is_authorized());
    }

    // The plane separation in the routing loop: a feasibility/quality re-route
    // excludes all cloud arms unless the policy permits silent cloud.
    #[spec("ROUTER-021")]
    #[test]
    fn reroute_excludes_cloud_unless_policy_permits() {
        use crate::decision::ModelChoice;

        for policy in [CloudPolicy::LocalOnly, CloudPolicy::AskBeforeCloud] {
            let excluded = augment_exclusions_for_policy(Vec::new(), policy);
            for cloud in ModelChoice::ALL_CLOUD {
                assert!(
                    excluded.iter().any(|e| e == cloud.name()),
                    "{policy:?} must exclude cloud arm {}",
                    cloud.name()
                );
            }
            // Local arms must remain selectable.
            assert!(
                !excluded.iter().any(|e| e == ModelChoice::LocalQwen3.name()),
                "local arms must not be excluded"
            );
        }

        // CloudWithinCap leaves the set untouched — cloud stays selectable.
        let excluded = augment_exclusions_for_policy(Vec::new(), CloudPolicy::CloudWithinCap);
        assert!(excluded.is_empty());
    }

    #[test]
    fn augment_preserves_existing_exclusions_without_duplicating() {
        use crate::decision::ModelChoice;
        let seed = vec![
            ModelChoice::LocalQwen3.name().to_string(),
            ModelChoice::Glm52.name().to_string(),
        ];
        let excluded = augment_exclusions_for_policy(seed, CloudPolicy::AskBeforeCloud);
        // Glm52 was already present; it must appear exactly once.
        let glm_count = excluded
            .iter()
            .filter(|e| *e == ModelChoice::Glm52.name())
            .count();
        assert_eq!(glm_count, 1, "existing cloud exclusion must not duplicate");
        // The seeded local exclusion is preserved.
        assert!(excluded.iter().any(|e| e == ModelChoice::LocalQwen3.name()));
    }

    // Regression: budget exhaustion keeps us local even when the user confirmed
    // and a collapse occurred. Budget is the sole spend authority.
    #[test]
    fn exhausted_budget_stays_local() {
        let caps = SpendCaps {
            remaining_cap: TokenCost::from_cents(1),
            per_request_max: None,
        };
        let decision = authorize_upgrade(
            CloudPolicy::CloudWithinCap,
            CloudSpendReason::LocalCollapsed,
            TokenCost::from_dollars(0.5),
            caps,
            true,
        );
        assert!(!decision.is_authorized());
    }
}
