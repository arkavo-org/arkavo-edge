//! What a chat session does when routing refuses an unconfirmed cloud call.
//!
//! Nothing here touches a console. The question is put through the host's own
//! [`CloudConsentPrompt`], so a session served by a process with no user to ask
//! — an A2A server answering a remote client — gets the refusal back
//! immediately instead of blocking on the server's stdin. An approval is
//! recorded against this session's id alone, so the next session the same
//! process serves is asked its own question.

use arkavo_router::{CloudConsentPrompt, CloudConsentRequest, Router};
use std::sync::Arc;

/// What the chat loop does when routing refuses an unconfirmed cloud call.
#[derive(Debug, Clone, PartialEq)]
pub enum CloudConfirmation {
    /// Surface the router error unchanged: it is not a confirmation refusal,
    /// the host has no channel to ask on, or the user has already answered
    /// this session — yes or no — and asking again would only repeat the
    /// question.
    Propagate,
    /// Ask once, then retry the identical request if the user agrees.
    Ask {
        model: String,
        estimated_cost_usd: f64,
    },
}

/// Whether this routing failure is a question for the user.
///
/// Cloud augmentation under `AskBeforeCloud` needs approval: the router
/// refuses automatic cloud selection and has no channel to reach the user, so
/// the chat loop asks through the host's prompter and re-dispatches the same
/// request.
///
/// `already_asked` covers both answers. A user who declined has answered the
/// question for this session, so repeating it every turn would be nagging, not
/// recovery; the original error propagates instead.
pub fn cloud_confirmation(
    error: &arkavo_router::Error,
    has_prompt: bool,
    already_asked: bool,
) -> CloudConfirmation {
    match error {
        arkavo_router::Error::CloudConfirmationRequired {
            model,
            estimated_cost_usd,
        } if has_prompt && !already_asked => CloudConfirmation::Ask {
            model: model.clone(),
            estimated_cost_usd: *estimated_cost_usd,
        },
        _ => CloudConfirmation::Propagate,
    }
}

/// Ask the host, and record a yes against this session id.
///
/// The approval is standing, so the calls this turn fans out into — and every
/// later turn of the same conversation — inherit it without re-asking.
///
/// The approval never reaches another session, and it is never persisted: it
/// lives on the running router and dies with the process.
pub async fn request_cloud_consent(
    prompt: &Arc<dyn CloudConsentPrompt>,
    router: &Router,
    session_id: &str,
    model: &str,
    estimated_cost_usd: f64,
) -> bool {
    let approved = prompt
        .ask(CloudConsentRequest::Call {
            model,
            estimated_cost_usd,
        })
        .await;
    if approved {
        router.approve_cloud_for_session(session_id);
    }
    approved
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// A prompter that answers a fixed way and counts how often it was asked.
    struct ScriptedPrompt {
        answer: bool,
        asked: std::sync::atomic::AtomicUsize,
    }

    impl ScriptedPrompt {
        fn new(answer: bool) -> Self {
            Self {
                answer,
                asked: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl CloudConsentPrompt for ScriptedPrompt {
        async fn ask(&self, _request: CloudConsentRequest<'_>) -> bool {
            self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.answer
        }
    }

    fn needs_cloud_confirmation() -> arkavo_router::Error {
        arkavo_router::Error::CloudConfirmationRequired {
            model: "gpt-6-astra".to_string(),
            estimated_cost_usd: 0.0123,
        }
    }

    #[spec("ASTRA-004")]
    #[test]
    fn cloud_confirmation_asks_at_most_once_per_session() {
        let needs_confirmation = needs_cloud_confirmation();
        assert_eq!(
            cloud_confirmation(&needs_confirmation, true, false),
            CloudConfirmation::Ask {
                model: "gpt-6-astra".to_string(),
                estimated_cost_usd: 0.0123,
            }
        );
        // Regression: a host with no channel to its user — every A2A server —
        // keeps the error path and never blocks waiting for an answer.
        assert_eq!(
            cloud_confirmation(&needs_confirmation, false, false),
            CloudConfirmation::Propagate
        );
        // Already asked this session: never ask twice, never loop. This holds
        // for an approval (the router carries it) and for a decline.
        assert_eq!(
            cloud_confirmation(&needs_confirmation, true, true),
            CloudConfirmation::Propagate
        );
        // Any other routing failure is untouched.
        assert_eq!(
            cloud_confirmation(
                &arkavo_router::Error::ModelExecution("boom".into()),
                true,
                false
            ),
            CloudConfirmation::Propagate
        );
    }

    /// Replays the session flag across turns: ask, decline, then a second turn
    /// that hits the same refusal must not put the question again.
    #[spec("ASTRA-004")]
    #[test]
    fn a_declined_session_is_never_asked_again() {
        let error = needs_cloud_confirmation();
        let mut cloud_asked = false;

        assert!(matches!(
            cloud_confirmation(&error, true, cloud_asked),
            CloudConfirmation::Ask { .. }
        ));
        // The loop marks the question as put before reading the answer, so a
        // decline is recorded exactly as an approval is.
        cloud_asked = true;

        assert_eq!(
            cloud_confirmation(&error, true, cloud_asked),
            CloudConfirmation::Propagate,
            "a declined session must not be re-prompted on the next turn"
        );
    }

    /// After a yes the router holds the approval for the session, so later turns
    /// should not refuse at all — but if one still does, the loop must surface
    /// the error rather than putting the question a second or third time.
    #[spec("ASTRA-004")]
    #[test]
    fn an_approved_session_is_never_asked_again_across_turns() {
        let error = needs_cloud_confirmation();
        let mut cloud_asked = false;

        assert!(matches!(
            cloud_confirmation(&error, true, cloud_asked),
            CloudConfirmation::Ask { .. }
        ));
        cloud_asked = true;

        for turn in 1..=2 {
            assert_eq!(
                cloud_confirmation(&error, true, cloud_asked),
                CloudConfirmation::Propagate,
                "turn {turn} after an approval must not re-prompt"
            );
        }
    }

    /// Regression: an approval reaches the session that gave it and no other.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn an_approval_is_recorded_against_one_session_only() {
        let router = Router::new_offline().await.expect("router");
        let prompt: Arc<dyn CloudConsentPrompt> = Arc::new(ScriptedPrompt::new(true));

        assert!(
            request_cloud_consent(&prompt, &router, "session-a", "gpt-6-astra", 0.0123).await,
            "an approving host authorizes the call"
        );
        assert!(router.cloud_approved(Some("session-a")));
        assert!(!router.cloud_approved(Some("session-b")));
        assert!(!router.cloud_approved(None));
    }

    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_declined_request_records_nothing() {
        let router = Router::new_offline().await.expect("router");
        let scripted = Arc::new(ScriptedPrompt::new(false));
        let prompt: Arc<dyn CloudConsentPrompt> = scripted.clone();

        assert!(!request_cloud_consent(&prompt, &router, "session-a", "gpt-6-astra", 0.0123).await);
        assert_eq!(scripted.asked(), 1);
        assert!(!router.cloud_approved(Some("session-a")));
    }
}
