use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::{Result, ensure};
use arkavo_budget::{
    BudgetTracker, CloudSpendReason, CloudSpendRequest, SpendCaps, TokenCost,
    authorize_cloud_spend, cost::TokenUsage,
};
use tokio::sync::watch;

use crate::{CodexConfig, RunOutcome, SessionBinding, SpendApproval, Usage, process, store::Store};

/// One host-authorized coding session. A separate state file is required per job.
pub struct CodexWorker {
    config: CodexConfig,
    approval: SpendApproval,
    budget: Arc<BudgetTracker>,
    store: Mutex<Store>,
    active: tokio::sync::Mutex<()>,
    cancel: watch::Sender<bool>,
    confirmed: std::sync::atomic::AtomicBool,
}

impl CodexWorker {
    /// State belongs to the trusted orchestrator and must live outside the workspace.
    pub fn open(
        mut config: CodexConfig,
        state_path: &Path,
        approval: SpendApproval,
        budget: Arc<BudgetTracker>,
    ) -> Result<Self> {
        config.validate(&approval)?;
        let store = Store::open(state_path, &config)?;
        let (cancel, _) = watch::channel(false);
        let confirmed = std::sync::atomic::AtomicBool::new(approval.user_confirmed);
        Ok(Self {
            config,
            approval,
            budget,
            store: Mutex::new(store),
            active: tokio::sync::Mutex::new(()),
            cancel,
            confirmed,
        })
    }

    pub fn session(&self) -> Result<SessionBinding> {
        Ok(self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("Session lock poisoned"))?
            .binding
            .clone())
    }

    fn update(&self, f: impl FnOnce(&mut SessionBinding)) -> Result<()> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("Session lock poisoned"))?;
        f(&mut store.binding);
        store.save()
    }

    /// Takes ownership of the Codex thread itself, not merely of this state
    /// file, so a copied state file cannot drive the same remote session.
    fn bind_thread(&self, id: &str) -> Result<()> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("Session lock poisoned"))?
            .bind_thread(id)
    }

    /// Cancel is independent of the run lock, including while stdout is idle.
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }

    pub async fn run(&self, prompt: &str) -> Result<RunOutcome> {
        let _active = self
            .active
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Worker is already running"))?;
        ensure!(
            !prompt.trim().is_empty() && prompt.len() <= 1024 * 1024,
            "Prompt must contain 1..1048576 bytes"
        );
        let session = self.session()?;
        ensure!(
            !session.accounting_incomplete,
            "Previous attempt has unaccounted usage; host reconciliation is required"
        );
        let remaining = self
            .budget
            .get_status()
            .await
            .session_remaining
            .unwrap_or_else(|| TokenCost::from_cents(u64::MAX));
        let verdict = authorize_cloud_spend(
            self.approval.policy,
            &CloudSpendRequest {
                reason: CloudSpendReason::UserRequested,
                projected_cost: self.approval.projected_cost,
                user_confirmed: self.confirmed.load(std::sync::atomic::Ordering::Relaxed),
            },
            SpendCaps {
                remaining_cap: remaining,
                per_request_max: None,
            },
        );
        ensure!(
            verdict.is_authorized(),
            "Codex cloud spend is not authorized: {verdict:?}"
        );
        ensure!(
            self.budget
                .can_afford(&self.config.agent_id, self.approval.projected_cost)
                .await?,
            "Codex admission estimate exceeds the shared budget"
        );
        self.cancel.send_replace(false);
        // Persist before spawning: a host crash must not erase a possible charge.
        self.update(|binding| binding.accounting_incomplete = true)?;
        let result = process::run(
            &self.config,
            session.thread_id.as_deref(),
            prompt,
            self.cancel.subscribe(),
            |id| self.bind_thread(id),
        )
        .await;
        let mut outcome = match result {
            Ok(outcome) => {
                // Codex was spawned and given the prompt, so the attempt has
                // consumed the one-shot confirmation whatever its status.
                self.confirmed
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                outcome
            }
            Err(e) => {
                // Spawn failed before Codex could accept a prompt: neither the
                // confirmation nor the accounting flag may be burnt by it.
                self.update(|binding| binding.accounting_incomplete = false)?;
                return Err(e);
            }
        };
        if let Some(usage) = &outcome.usage {
            let dollars = self.cost(usage);
            self.record(usage, dollars).await?;
            outcome.estimated_cost_usd = Some(dollars);
            self.update(|binding| binding.accounting_incomplete = false)?;
        }
        Ok(outcome)
    }

    fn cost(&self, usage: &Usage) -> f64 {
        let pricing = &self.approval.pricing;
        // Codex reports cached and cache-written tokens inside `input_tokens`,
        // and each of the three rates is a total for its own bucket, so the
        // buckets are made disjoint before any of them is priced. This matches
        // `calculate_cost` in arkavo-router, which the ledger is shared with.
        let ordinary = f64::from(
            usage
                .input_tokens
                .saturating_sub(usage.cached_input_tokens)
                .saturating_sub(usage.cache_write_input_tokens),
        );
        let cached_rate = pricing
            .cached_input_cents_per_mtok
            .unwrap_or(pricing.input_cents_per_mtok) as f64;
        // An unpriced cache write is ordinary input, never free.
        let write_rate = pricing
            .cache_write_cents_per_mtok
            .unwrap_or(pricing.input_cents_per_mtok) as f64;
        let generated = f64::from(usage.output_tokens).mul_add(
            pricing.output_cents_per_mtok as f64,
            ordinary * pricing.input_cents_per_mtok as f64,
        );
        let read = f64::from(usage.cached_input_tokens).mul_add(cached_rate, generated);
        f64::from(usage.cache_write_input_tokens).mul_add(write_rate, read) / 100_000_000.0
    }

    async fn record(&self, usage: &Usage, dollars: f64) -> Result<()> {
        // Codex counts cached and freshly cached tokens inside `input_tokens`,
        // while the ledger totals its fields, so each token is attributed once.
        let tokens = TokenUsage {
            input_tokens: usage
                .input_tokens
                .saturating_sub(usage.cached_input_tokens)
                .saturating_sub(usage.cache_write_input_tokens),
            cached_input_tokens: usage.cached_input_tokens,
            output_tokens: usage.output_tokens - usage.reasoning_output_tokens,
            thinking_tokens: usage.reasoning_output_tokens,
            cache_write_tokens: usage.cache_write_input_tokens,
        };
        self.budget
            .record_spending_precise(
                self.config.agent_id.clone(),
                "openai".into(),
                self.config.model.clone(),
                tokens,
                dollars,
            )
            .await?;
        Ok(())
    }

    /// Host-only recovery using reconciled provider usage/cost. Never an LLM tool.
    /// A crash between ledger recording and state persistence may require the
    /// host to deduplicate against its durable ledger before calling this method.
    pub async fn reconcile(&self, usage: Usage, dollars: f64) -> Result<()> {
        let _active = self
            .active
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Worker is already running"))?;
        ensure!(
            self.session()?.accounting_incomplete,
            "No incomplete attempt to reconcile"
        );
        usage.validate()?;
        self.record(&usage, dollars).await?;
        self.update(|binding| binding.accounting_incomplete = false)
    }
}
