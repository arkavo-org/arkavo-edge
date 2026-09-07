use std::{path::PathBuf, time::Duration};

use anyhow::{Result, ensure};
use arkavo_budget::{CloudPolicy, PricingEntry, TokenCost};
use serde::{Deserialize, Serialize};

const DEFAULT_TIMEOUT_SECONDS: u64 = 1_800;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sandbox {
    #[default]
    ReadOnly,
    WorkspaceWrite,
}

impl Sandbox {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }
}

/// A trusted host decision, deliberately absent from MCP tool arguments.
#[derive(Debug, Clone)]
pub struct SpendApproval {
    pub policy: CloudPolicy,
    pub user_confirmed: bool,
    /// Admission estimate, not a hard cap on Codex's internal inference loop.
    pub projected_cost: TokenCost,
    pub pricing: PricingEntry,
}

#[derive(Debug, Clone)]
pub struct CodexConfig {
    pub executable: PathBuf,
    pub workspace: PathBuf,
    pub agent_id: String,
    pub model: String,
    pub sandbox: Sandbox,
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl CodexConfig {
    /// Discover Codex through PATH, without downloading or authenticating it.
    pub fn discover(workspace: PathBuf, agent_id: String) -> Result<Self> {
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let executable = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
            .ok_or_else(|| anyhow::anyhow!("Codex CLI is not installed on PATH"))?;
        Ok(Self {
            executable,
            workspace: workspace.canonicalize()?,
            agent_id,
            model: "gpt-6-astra".into(),
            sandbox: Sandbox::ReadOnly,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            max_output_bytes: 16 * 1024 * 1024,
        })
    }

    pub(crate) fn validate(&mut self, approval: &SpendApproval) -> Result<()> {
        self.workspace = self.workspace.canonicalize()?;
        self.executable = self.executable.canonicalize()?;
        ensure!(self.workspace.is_dir(), "Workspace must be a directory");
        ensure!(self.executable.is_file(), "Codex executable must be a file");
        ensure!(
            !self.agent_id.trim().is_empty(),
            "Agent identity is required"
        );
        ensure!(!self.model.trim().is_empty(), "Model is required");
        ensure!(!self.timeout.is_zero(), "Timeout must be positive");
        ensure!(
            (1..=64 * 1024 * 1024).contains(&self.max_output_bytes),
            "Output limit must be between one byte and 64 MiB"
        );
        ensure!(
            approval.pricing.model_id == self.model && approval.pricing.provider == "openai",
            "Explicit OpenAI pricing for the selected model is required"
        );
        ensure!(
            approval.pricing.input_cents_per_mtok > 0
                && approval.pricing.output_cents_per_mtok > 0
                && !approval.projected_cost.is_zero(),
            "Positive pricing and admission estimate are required"
        );
        Ok(())
    }
}
