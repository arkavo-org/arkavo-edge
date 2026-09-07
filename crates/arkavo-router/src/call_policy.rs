use crate::{Error, ModelChoice, Result, Router};
use arkavo_budget::{
    CloudPolicy, CloudSpendDecision, CloudSpendReason, CloudSpendRequest, SpendCaps, TokenCost,
    authorize_cloud_spend,
};

impl Router {
    /// Enforce cloud policy and spend caps before dispatching a provider call.
    ///
    /// `session` names whose standing approval this call may draw on — the
    /// chat session that asked, or `None` for work the host process does on
    /// its own behalf. An approval given in one session is invisible here to
    /// every other one.
    pub async fn authorize_call(
        &self,
        model: &ModelChoice,
        dollars: f64,
        explicit: bool,
        session: Option<&str>,
    ) -> Result<()> {
        if model.is_local() {
            return Ok(());
        }
        authorize(
            model,
            dollars,
            self.cloud_policy,
            self.offline_mode,
            explicit || self.cloud_confirmed(session),
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

    /// Regression: the approval used to be one atomic on the shared router, so
    /// the first session to say yes authorized every session the process
    /// served. Each session now answers for itself.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn one_session_approval_does_not_authorize_another_session() {
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "openai", &provider).await;
        router.approve_cloud_for_session("session-a");

        assert!(
            router
                .authorize_call(&ModelChoice::Gpt6Astra, 0.2, false, Some("session-a"))
                .await
                .is_ok(),
            "the session that approved must be authorized"
        );
        assert!(
            matches!(
                router
                    .authorize_call(&ModelChoice::Gpt6Astra, 0.2, false, Some("session-b"))
                    .await,
                Err(Error::CloudConfirmationRequired { .. })
            ),
            "another session must still be asked"
        );
        assert!(
            matches!(
                router
                    .authorize_call(&ModelChoice::Gpt6Astra, 0.2, false, None)
                    .await,
                Err(Error::CloudConfirmationRequired { .. })
            ),
            "the host's own work must still be asked"
        );
        assert_eq!(provider.calls(), 0, "authorization dispatches nothing");
    }

    /// The mirror: an operator's startup approval covers the host's own routing
    /// calls and never reaches a chat session.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_host_approval_authorizes_only_the_host() {
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "openai", &provider).await;
        router.approve_cloud_for_host();

        assert!(
            router
                .authorize_call(&ModelChoice::Gpt6Astra, 0.2, false, None)
                .await
                .is_ok()
        );
        assert!(matches!(
            router
                .authorize_call(&ModelChoice::Gpt6Astra, 0.2, false, Some("session-a"))
                .await,
            Err(Error::CloudConfirmationRequired { .. })
        ));
    }

    /// An explicit model choice — `--model` or a manifest `model:` hint — is
    /// itself the user's consent, so it needs no separate approval.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn an_explicit_model_choice_is_its_own_consent() {
        use crate::test_support::{CountingProvider, cloud_router};

        let provider = CountingProvider::new("ok");
        let router = cloud_router(CloudPolicy::AskBeforeCloud, "openai", &provider).await;
        assert!(
            router
                .authorize_call(&ModelChoice::Gpt6Astra, 0.2, true, Some("session-a"))
                .await
                .is_ok()
        );
    }
}
