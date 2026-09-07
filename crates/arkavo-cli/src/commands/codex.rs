use std::path::PathBuf;

use anyhow::Context;
use arkavo_mcp_codex::{CodexConfig, CodexWorker, RunStatus, Sandbox, Usage};
use clap::Parser;

#[derive(Parser)]
#[command(
    name = "arkavo codex",
    about = "Run a Codex worker using saved Codex authentication. Cost reporting is an API-price estimate, not a billing statement."
)]
struct Args {
    /// The task to delegate. Required for a run, and refused alongside
    /// `--acknowledge-unrecorded-spend`, which only repairs the session.
    #[arg(long, required_unless_present = "acknowledge_unrecorded_spend")]
    prompt: Option<String>,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// Resume the Edge session state file printed by a previous invocation.
    #[arg(long)]
    resume: Option<PathBuf>,
    /// Grant workspace writes to this coding worker.
    #[arg(long)]
    write: bool,
    /// Record the dollars an interrupted attempt actually cost, clearing the
    /// session's reconciliation mark so it can run again.
    ///
    /// This is the operator asserting what the provider billed: nothing in Edge
    /// observed that charge, which is exactly why the session refuses to run
    /// until a person supplies it. Passing `0` asserts the attempt cost nothing,
    /// and is a statement, not an assumption.
    #[arg(long, value_name = "DOLLARS", conflicts_with = "prompt")]
    acknowledge_unrecorded_spend: Option<f64>,
}

#[allow(
    clippy::disallowed_methods,
    reason = "This synchronous CLI entry point owns its runtime"
)]
pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let args = match Args::try_parse_from(
        std::iter::once("arkavo codex".to_string()).chain(args.iter().cloned()),
    ) {
        Ok(args) => args,
        Err(e)
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            e.print()?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    tokio::runtime::Runtime::new()?.block_on(run(args))?;
    Ok(())
}

/// Identity of the command's session, and so the name of its state file.
///
/// A workspace-write grant is a different session from a read-only one: the
/// store refuses a saved binding whose sandbox changed, so the two grant levels
/// must not compete for one file.
fn agent_id(write: bool) -> &'static str {
    if write {
        "arkavo-cli-codex-write"
    } else {
        "arkavo-cli-codex"
    }
}

async fn run(args: Args) -> anyhow::Result<()> {
    let mut config = CodexConfig::discover(args.workspace, agent_id(args.write).into())?;
    if args.write {
        config.sandbox = Sandbox::WorkspaceWrite;
    }
    // One identity in one workspace has one session file, named the way the
    // server names it. A per-invocation name would strand a `.json` and a
    // `.lock` on every run, and — worse — would hide a binding left marked for
    // reconciliation, which is exactly the record the next run must see.
    let state = if let Some(path) = args.resume {
        anyhow::ensure!(path.is_file(), "Resume state does not exist");
        path
    } else {
        arkavo_server::codex::state_path(&config.agent_id, &config.workspace)?
    };
    // One spend plane: the caps and posture this workspace's AGENTS.md declares,
    // and the router's price table for the model Codex runs. Admission is now a
    // real check against the configured cap rather than a fixed estimate.
    let budget = arkavo_server::budget_tracker_from_agents_md().await?;
    // Invocation of this command is an explicit, single-turn cloud request; the
    // worker consumes that confirmation after one attempt.
    let approval =
        arkavo_server::codex::spend_approval(arkavo_server::cloud_policy_from_agents_md(), true);
    let worker = CodexWorker::open(config, &state, approval, budget)?;
    eprintln!("Codex session state: {}", state.display());
    if let Some(dollars) = args.acknowledge_unrecorded_spend {
        return acknowledge(&worker, &state, dollars).await;
    }
    let Some(prompt) = args.prompt.as_deref() else {
        anyhow::bail!("A prompt is required unless --acknowledge-unrecorded-spend is given");
    };
    let result = worker.run(prompt);
    tokio::pin!(result);
    let outcome = tokio::select! {
        outcome = &mut result => outcome,
        signal = tokio::signal::ctrl_c() => {
            signal?;
            worker.cancel();
            result.await
        }
    }
    .with_context(|| {
        format!(
            "Codex session state: {}. A run interrupted before Codex reported usage leaves \
             the binding marked for reconciliation, and this session refuses further runs \
             until that charge is recorded: read the attempt's cost from the provider and \
             pass it to `arkavo codex --acknowledge-unrecorded-spend <DOLLARS>`.",
            state.display()
        )
    })?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    anyhow::ensure!(
        outcome.status == RunStatus::Completed,
        "Codex worker did not complete"
    );
    Ok(())
}

/// Record an operator-supplied charge against a session left marked for
/// reconciliation, and stop. Never a run: the point of the mark is that spend
/// went unrecorded, and starting work would put a second unmeasured attempt on
/// top of the first.
///
/// The token breakdown is left at zero because the operator reconciled a bill,
/// not a usage report; the ledger entry is denominated in the dollars they
/// supplied, which is the figure the caps are enforced against.
async fn acknowledge(
    worker: &CodexWorker,
    state: &std::path::Path,
    dollars: f64,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        dollars.is_finite() && dollars >= 0.0,
        "Acknowledged spend must be a non-negative dollar amount"
    );
    worker.reconcile(Usage::default(), dollars).await?;
    println!(
        "Recorded ${dollars:.2} of previously unrecorded spend and cleared the reconciliation \
         mark on {}. This session accepts runs again.",
        state.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_require_an_explicit_grant() {
        let args = Args::try_parse_from(["codex", "--prompt", "review"]).unwrap();
        assert!(!args.write);
        assert!(args.resume.is_none());
        assert!(
            Args::try_parse_from([
                "codex",
                "--prompt",
                "review",
                "--sandbox",
                "danger-full-access"
            ])
            .is_err()
        );
    }

    /// A fresh state file per invocation left a `.json` and a `.lock` behind on
    /// every run, and hid a binding awaiting reconciliation from the next one.
    /// The name is now derived from identity and workspace, and the two grant
    /// levels are separate identities because the store refuses a binding whose
    /// sandbox changed.
    #[test]
    fn repeated_invocations_reuse_one_state_file_per_grant_level() {
        let workspace = std::path::Path::new("/codex-state-name-fixture");
        let read_only = arkavo_server::codex::state_path(agent_id(false), workspace).unwrap();
        let write = arkavo_server::codex::state_path(agent_id(true), workspace).unwrap();
        assert_eq!(
            read_only,
            arkavo_server::codex::state_path(agent_id(false), workspace).unwrap()
        );
        assert_ne!(read_only, write);
        assert_ne!(
            read_only,
            arkavo_server::codex::state_path(agent_id(false), std::path::Path::new("/elsewhere"))
                .unwrap()
        );
    }

    /// Recovering a session is its own invocation: it records a charge and
    /// stops. Combining it with a prompt would start a second attempt on top of
    /// an unmeasured one, so clap refuses the pair outright.
    #[test]
    fn acknowledging_unrecorded_spend_is_its_own_invocation() {
        let args = Args::try_parse_from(["codex", "--acknowledge-unrecorded-spend", "1.25"])
            .expect("acknowledgement needs no prompt");
        assert_eq!(args.acknowledge_unrecorded_spend, Some(1.25));
        assert!(args.prompt.is_none());
        // Asserting that the attempt cost nothing is still an assertion.
        assert!(Args::try_parse_from(["codex", "--acknowledge-unrecorded-spend", "0"]).is_ok());
        // A run still needs a prompt, and the two modes never combine.
        assert!(Args::try_parse_from(["codex"]).is_err());
        assert!(
            Args::try_parse_from([
                "codex",
                "--prompt",
                "review",
                "--acknowledge-unrecorded-spend",
                "1.25"
            ])
            .is_err()
        );
    }

    /// The command's spend authority: the router's rate card for the model the
    /// worker runs, a positive admission estimate, and the single confirmation
    /// that invoking the command represents. The worker rejects an approval
    /// whose pricing names another model or provider.
    #[test]
    fn invocation_carries_router_pricing_and_one_confirmation() {
        let approval =
            arkavo_server::codex::spend_approval(arkavo_budget::CloudPolicy::AskBeforeCloud, true);
        assert!(approval.user_confirmed);
        assert!(!approval.projected_cost.is_zero());
        assert_eq!(approval.pricing.provider, "openai");
        assert_eq!(approval.pricing.model_id, "gpt-6-astra");
        assert!(approval.pricing.input_cents_per_mtok > 0);
        assert!(approval.pricing.output_cents_per_mtok > 0);
    }
}
