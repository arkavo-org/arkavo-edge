use crate::{Error, ModelChoice, Result, Router};
use arkavo_budget::{
    CloudPolicy, CloudSpendDecision, CloudSpendReason, CloudSpendRequest, SpendCaps, TokenCost,
    authorize_cloud_spend,
};

impl Router {
    pub(crate) async fn authorize_call(
        &self,
        model: &ModelChoice,
        dollars: f64,
        explicit: bool,
    ) -> Result<()> {
        if model.is_local() {
            return Ok(());
        }
        authorize(
            model,
            dollars,
            self.cloud_policy,
            self.offline_mode,
            explicit || self.cloud_confirmed(),
            self.cloud_spend_caps().await,
        )
    }
}

fn authorize(
    model: &ModelChoice,
    dollars: f64,
    policy: CloudPolicy,
    offline: bool,
    confirmed: bool,
    caps: SpendCaps,
) -> Result<()> {
    if offline {
        return Err(Error::ModerationBlocked {
            policy_id: "offline".into(),
            reason: "Cloud inference is disabled in offline mode".into(),
        });
    }
    let request = CloudSpendRequest {
        reason: CloudSpendReason::UserRequested,
        projected_cost: TokenCost::from_cents((dollars * 100.0).ceil() as u64),
        user_confirmed: confirmed,
    };
    match authorize_cloud_spend(policy, &request, caps) {
        CloudSpendDecision::Authorized { .. } => Ok(()),
        CloudSpendDecision::NeedsUserConfirmation { .. } => Err(Error::CloudConfirmationRequired {
            model: model.name().into(),
            estimated_cost_usd: dollars,
        }),
        CloudSpendDecision::Denied(reason) => Err(Error::ModerationBlocked {
            policy_id: "cloud_spend".into(),
            reason: format!("Cloud inference denied: {reason:?}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn caps() -> SpendCaps {
        SpendCaps {
            remaining_cap: TokenCost::from_dollars(10.0),
            per_request_max: None,
        }
    }

    #[spec("ASTRA-004")]
    #[test]
    fn explicit_cloud_selection_still_respects_local_only() {
        assert!(matches!(
            authorize(
                &ModelChoice::Gpt6Astra,
                0.2,
                CloudPolicy::LocalOnly,
                false,
                true,
                caps()
            ),
            Err(Error::ModerationBlocked { .. })
        ));
    }

    #[spec("ASTRA-004")]
    #[test]
    fn automatic_cloud_requires_confirmation() {
        assert!(matches!(
            authorize(
                &ModelChoice::Gpt6Astra,
                0.2,
                CloudPolicy::AskBeforeCloud,
                false,
                false,
                caps()
            ),
            Err(Error::CloudConfirmationRequired { .. })
        ));
        assert!(
            authorize(
                &ModelChoice::Gpt6Astra,
                0.2,
                CloudPolicy::AskBeforeCloud,
                false,
                true,
                caps()
            )
            .is_ok()
        );
    }

    #[spec("ASTRA-004")]
    #[test]
    fn explicit_cloud_cannot_bypass_budget_or_offline() {
        assert!(
            authorize(
                &ModelChoice::Gpt6Astra,
                20.0,
                CloudPolicy::AskBeforeCloud,
                false,
                true,
                caps()
            )
            .is_err()
        );
        assert!(
            authorize(
                &ModelChoice::Gpt6Astra,
                0.2,
                CloudPolicy::CloudWithinCap,
                true,
                true,
                caps()
            )
            .is_err()
        );
    }
}
