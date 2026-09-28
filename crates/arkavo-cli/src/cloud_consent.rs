//! The terminal's answer to "may this spend on cloud inference?".
//!
//! This is the only place in the workspace that reads a console for consent.
//! Library crates take a [`CloudConsentPrompt`] and never assume one exists, so
//! a process with no operator — an A2A server answering a remote client — is
//! refused immediately instead of waiting on stdin nobody is watching.

use arkavo_router::{CloudConsentPrompt, CloudConsentRequest};
use std::io::IsTerminal;

/// Puts the y/N question on the controlling terminal.
///
/// A run with no terminal declines without asking, which leaves the router's
/// policy error as the outcome — an unattended run must not spend against a
/// prompt nobody answered.
pub struct TtyCloudConsent;

impl TtyCloudConsent {
    pub fn new() -> Self {
        Self
    }

    /// Whether a question could be put right now.
    pub fn is_interactive() -> bool {
        std::io::stdin().is_terminal()
    }
}

impl Default for TtyCloudConsent {
    fn default() -> Self {
        Self::new()
    }
}

/// The exact wording each question uses, kept out of the I/O so it can be
/// asserted without a terminal.
fn question(request: CloudConsentRequest<'_>) -> String {
    match request {
        CloudConsentRequest::Call {
            model,
            estimated_cost_usd,
        } => format!(
            "\nCloud inference with {model} is estimated at ${estimated_cost_usd:.4} for this request.\nSend this session's requests to the cloud? [y/N]: "
        ),
        CloudConsentRequest::Session => concat!(
            "\nCloud models are configured and cloud policy is ask-before-cloud.\n",
            "Allow this agent to augment its local inference with paid cloud inference ",
            "for this session? [y/N]: "
        )
        .to_string(),
    }
}

/// Anything but an explicit yes declines.
fn approves(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[async_trait::async_trait]
impl CloudConsentPrompt for TtyCloudConsent {
    async fn ask(&self, request: CloudConsentRequest<'_>) -> bool {
        if !Self::is_interactive() {
            return false;
        }
        let question = question(request);
        // The blocking read is kept off the async runtime; the answer decides
        // whether this request can run at all, so nothing else proceeds until
        // it arrives.
        tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, Write};
            let mut stdout = std::io::stdout();
            if stdout.write_all(question.as_bytes()).is_err() || stdout.flush().is_err() {
                return false;
            }
            let mut answer = String::new();
            if std::io::stdin().lock().read_line(&mut answer).is_err() {
                return false;
            }
            approves(&answer)
        })
        .await
        .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// The wording a chat turn shows is unchanged from the prompt this
    /// replaced, down to the four-decimal cost.
    #[spec("ASTRA-004")]
    #[test]
    fn a_refused_call_names_the_model_and_the_cost() {
        let text = question(CloudConsentRequest::Call {
            model: "gpt-6-astra",
            estimated_cost_usd: 0.0123,
        });
        assert!(text.contains("Cloud inference with gpt-6-astra"));
        assert!(text.contains("$0.0123"));
        assert!(text.ends_with("[y/N]: "));
    }

    #[spec("ASTRA-004")]
    #[test]
    fn a_startup_question_asks_about_the_whole_session() {
        let text = question(CloudConsentRequest::Session);
        assert!(text.contains("ask-before-cloud"));
        assert!(text.ends_with("[y/N]: "));
    }

    #[spec("ASTRA-004")]
    #[test]
    fn only_an_explicit_yes_approves() {
        for yes in ["y", "Y", "yes", "YES\n", " yes \n"] {
            assert!(approves(yes), "{yes:?} must approve");
        }
        for no in ["", "\n", "n", "no", "maybe", "ye"] {
            assert!(!approves(no), "{no:?} must decline");
        }
    }

    /// A run with no terminal answers "no" rather than waiting: an unattended
    /// process must fall back to the router's policy error.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_non_interactive_run_declines_without_reading_stdin() {
        if TtyCloudConsent::is_interactive() {
            // A developer running the suite from a terminal; the branch under
            // test is the non-interactive one, which CI always exercises.
            return;
        }
        assert!(
            !TtyCloudConsent::new()
                .ask(CloudConsentRequest::Session)
                .await
        );
    }
}
