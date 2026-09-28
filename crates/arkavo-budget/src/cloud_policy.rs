//! Cloud spend permission policy — the **spend plane** authority.
//!
//! Architectural rule:
//!   Availability changes what is *possible*.
//!   Quality changes what is *advisable*.
//!   Budget policy changes what is *allowed*.
//!
//! Only this module authorizes cloud spend. Feasibility/availability signals
//! (slow laptop, timeout, low tokens/sec, OOM) are deliberately NOT inputs
//! here: a busy local runtime is not a budget-authorization event. Quality
//! inadequacy may *request* cloud spend; only budget policy may *authorize* it.

use crate::cost::TokenCost;
use serde::{Deserialize, Serialize};

/// User-chosen posture for cloud spending.
///
/// The safe v1 default is [`CloudPolicy::AskBeforeCloud`]. `CloudWithinCap`
/// should only become a default after the adequacy gate is validated against
/// real user outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloudPolicy {
    /// Never call a paid cloud model. Stay local even if the local answer
    /// visibly collapses.
    LocalOnly,
    /// Cloud allowed only after explicit per-request user confirmation.
    #[default]
    AskBeforeCloud,
    /// Cloud allowed automatically while projected spend stays within the cap.
    /// Reserved for after the adequacy gate is validated.
    CloudWithinCap,
}

impl CloudPolicy {
    /// Parse a policy from a SwarmKit config `cloud_policy` string. Accepts snake_case,
    /// kebab-case, and a few natural aliases. Returns `None` for unknown input
    /// so the caller can keep the safe default.
    pub fn parse(s: &str) -> Option<Self> {
        match s
            .trim()
            .to_ascii_lowercase()
            .replace(['-', ' '], "_")
            .as_str()
        {
            "local_only" | "local" | "offline" => Some(Self::LocalOnly),
            "ask_before_cloud" | "ask" | "prompt" => Some(Self::AskBeforeCloud),
            "cloud_within_cap" | "cloud" | "auto" | "within_cap" => Some(Self::CloudWithinCap),
            _ => None,
        }
    }

    /// Whether the router may route to a paid cloud model *without first asking*
    /// the user. Only `CloudWithinCap` permits silent cloud spend; `LocalOnly`
    /// and `AskBeforeCloud` require staying local (or an explicit confirmation
    /// step) so a feasibility/quality signal can never silently spend.
    pub fn permits_silent_cloud(self) -> bool {
        matches!(self, Self::CloudWithinCap)
    }
}

/// Why a cloud upgrade is being requested.
///
/// A request is a *question*, never an authorization. There is deliberately no
/// `LocalSlow` / `LocalUnavailable` variant: feasibility failures may not
/// request spend. A slow laptop or a timeout is not a reason to spend money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudSpendReason {
    /// The user explicitly asked to use cloud.
    UserRequested,
    /// The local answer visibly collapsed (empty/truncated/loop/...).
    LocalCollapsed,
    /// The task class is high-risk and warrants a stronger model.
    HighRiskTask,
}

/// A request to spend cloud budget. Carries the projected per-request cost and
/// whether the user has confirmed this specific spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudSpendRequest {
    pub reason: CloudSpendReason,
    /// Projected cost of the cloud call (the per-request reservation amount).
    pub projected_cost: TokenCost,
    /// Whether the user has confirmed this specific spend.
    pub user_confirmed: bool,
}

/// Spend-plane caps that bound a single cloud call.
#[derive(Debug, Clone, Copy)]
pub struct SpendCaps {
    /// Cloud budget still available in the binding window.
    pub remaining_cap: TokenCost,
    /// Maximum a single request may cost. `None` = no per-request cap.
    pub per_request_max: Option<TokenCost>,
}

/// Why a cloud spend was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Policy forbids cloud entirely.
    LocalOnlyPolicy,
    /// Projected cost exceeds the remaining budget cap.
    BudgetExhausted,
    /// Projected cost exceeds the per-request maximum.
    PerRequestCapExceeded,
}

/// The spend plane's verdict on a cloud spend request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloudSpendDecision {
    /// Spend is authorized; reserve `cost`.
    Authorized { cost: TokenCost },
    /// Spend is permitted by budget but requires explicit user confirmation.
    NeedsUserConfirmation { projected_cost: TokenCost },
    /// Spend is denied.
    Denied(DenyReason),
}

impl CloudSpendDecision {
    pub fn is_authorized(&self) -> bool {
        matches!(self, Self::Authorized { .. })
    }
}

/// Authorize (or refuse) a cloud spend request — the sole spend authority.
///
/// The evaluation order encodes the architectural rule: a hardware/feasibility
/// signal never appears here, so it can never authorize spend. The decision is
/// a pure function of policy, the request, and the budget caps.
pub fn authorize_cloud_spend(
    policy: CloudPolicy,
    request: &CloudSpendRequest,
    caps: SpendCaps,
) -> CloudSpendDecision {
    // 1. Policy gate. LocalOnly forbids cloud regardless of everything else.
    if policy == CloudPolicy::LocalOnly {
        return CloudSpendDecision::Denied(DenyReason::LocalOnlyPolicy);
    }

    // 2. Per-request cap.
    if let Some(max) = caps.per_request_max
        && request.projected_cost > max
    {
        return CloudSpendDecision::Denied(DenyReason::PerRequestCapExceeded);
    }

    // 3. Remaining budget cap. Budget is the sole authority on "allowed".
    if request.projected_cost > caps.remaining_cap {
        return CloudSpendDecision::Denied(DenyReason::BudgetExhausted);
    }

    // 4. Confirmation gate. AskBeforeCloud needs explicit confirmation;
    //    CloudWithinCap authorizes silently within the cap.
    match policy {
        CloudPolicy::AskBeforeCloud => {
            if request.user_confirmed {
                CloudSpendDecision::Authorized {
                    cost: request.projected_cost,
                }
            } else {
                CloudSpendDecision::NeedsUserConfirmation {
                    projected_cost: request.projected_cost,
                }
            }
        }
        CloudPolicy::CloudWithinCap => CloudSpendDecision::Authorized {
            cost: request.projected_cost,
        },
        // Handled by the policy gate above.
        CloudPolicy::LocalOnly => CloudSpendDecision::Denied(DenyReason::LocalOnlyPolicy),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn caps(remaining_dollars: f64, per_request_dollars: Option<f64>) -> SpendCaps {
        SpendCaps {
            remaining_cap: TokenCost::from_dollars(remaining_dollars),
            per_request_max: per_request_dollars.map(TokenCost::from_dollars),
        }
    }

    fn request(confirmed: bool) -> CloudSpendRequest {
        CloudSpendRequest {
            reason: CloudSpendReason::LocalCollapsed,
            projected_cost: TokenCost::from_dollars(0.10),
            user_confirmed: confirmed,
        }
    }

    #[test]
    fn default_policy_is_ask_before_cloud() {
        assert_eq!(CloudPolicy::default(), CloudPolicy::AskBeforeCloud);
    }

    #[test]
    fn parse_accepts_aliases_and_rejects_unknown() {
        assert_eq!(
            CloudPolicy::parse("local_only"),
            Some(CloudPolicy::LocalOnly)
        );
        assert_eq!(
            CloudPolicy::parse("Local-Only"),
            Some(CloudPolicy::LocalOnly)
        );
        assert_eq!(
            CloudPolicy::parse("ask before cloud"),
            Some(CloudPolicy::AskBeforeCloud)
        );
        assert_eq!(
            CloudPolicy::parse("cloud_within_cap"),
            Some(CloudPolicy::CloudWithinCap)
        );
        assert_eq!(
            CloudPolicy::parse("auto"),
            Some(CloudPolicy::CloudWithinCap)
        );
        assert_eq!(CloudPolicy::parse("nonsense"), None);
    }

    #[test]
    fn only_cloud_within_cap_permits_silent_cloud() {
        assert!(!CloudPolicy::LocalOnly.permits_silent_cloud());
        assert!(!CloudPolicy::AskBeforeCloud.permits_silent_cloud());
        assert!(CloudPolicy::CloudWithinCap.permits_silent_cloud());
    }

    #[spec("BUDGET-011")]
    #[test]
    fn local_only_always_denies_even_when_confirmed_and_within_budget() {
        let decision =
            authorize_cloud_spend(CloudPolicy::LocalOnly, &request(true), caps(100.0, None));
        assert_eq!(
            decision,
            CloudSpendDecision::Denied(DenyReason::LocalOnlyPolicy)
        );
    }

    #[test]
    fn ask_before_cloud_needs_confirmation_even_with_budget() {
        let decision = authorize_cloud_spend(
            CloudPolicy::AskBeforeCloud,
            &request(false),
            caps(100.0, None),
        );
        assert_eq!(
            decision,
            CloudSpendDecision::NeedsUserConfirmation {
                projected_cost: TokenCost::from_dollars(0.10)
            }
        );
    }

    #[spec("BUDGET-011")]
    #[test]
    fn ask_before_cloud_authorizes_once_confirmed() {
        let decision = authorize_cloud_spend(
            CloudPolicy::AskBeforeCloud,
            &request(true),
            caps(100.0, None),
        );
        assert_eq!(
            decision,
            CloudSpendDecision::Authorized {
                cost: TokenCost::from_dollars(0.10)
            }
        );
    }

    #[test]
    fn cloud_within_cap_authorizes_without_confirmation() {
        let decision = authorize_cloud_spend(
            CloudPolicy::CloudWithinCap,
            &request(false),
            caps(100.0, None),
        );
        assert!(decision.is_authorized());
    }

    // Regression: budget is the sole authority. An exhausted cap denies even
    // CloudWithinCap, and even an explicitly confirmed AskBeforeCloud request.
    #[test]
    fn exhausted_budget_denies_regardless_of_policy() {
        for policy in [CloudPolicy::AskBeforeCloud, CloudPolicy::CloudWithinCap] {
            let decision = authorize_cloud_spend(policy, &request(true), caps(0.05, None));
            assert_eq!(
                decision,
                CloudSpendDecision::Denied(DenyReason::BudgetExhausted),
                "policy {policy:?} must defer to the budget cap"
            );
        }
    }

    #[test]
    fn per_request_cap_denies_oversized_call() {
        let decision = authorize_cloud_spend(
            CloudPolicy::CloudWithinCap,
            &request(true),
            caps(100.0, Some(0.05)),
        );
        assert_eq!(
            decision,
            CloudSpendDecision::Denied(DenyReason::PerRequestCapExceeded)
        );
    }

    // Regression: the per-request cap is checked before the remaining cap so
    // the most specific limit wins the denial reason.
    #[test]
    fn per_request_cap_takes_precedence_over_remaining_cap() {
        let decision = authorize_cloud_spend(
            CloudPolicy::AskBeforeCloud,
            &request(true),
            caps(0.01, Some(0.05)),
        );
        assert_eq!(
            decision,
            CloudSpendDecision::Denied(DenyReason::PerRequestCapExceeded)
        );
    }
}
