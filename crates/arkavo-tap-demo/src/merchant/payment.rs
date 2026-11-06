//! Payment states and the simulated provider.
//!
//! An order moves `prepared → authorized → submitted`, then to an outcome. A real provider's
//! outcomes are `confirmed`, `failed` or `outcome_unknown`; `confirmed` needs provider
//! evidence. The simulated provider never produces `confirmed`: its terminal state is
//! `simulated`, so a simulated purchase can never be reported as a settled payment.
//!
//! Submission is keyed by a stable provider operation reference, recorded before the call,
//! so a process that crashes after submitting reconciles by looking the reference up rather
//! than paying again.

use sqlx::Row as _;
use sqlx::SqlitePool;
use std::future::Future;

/// Where a payment stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentState {
    Prepared,
    Authorized,
    Submitted,
    Confirmed,
    Failed,
    OutcomeUnknown,
    /// The simulated provider's only outcome. Not a payment.
    Simulated,
}

impl PaymentState {
    pub fn as_str(self) -> &'static str {
        match self {
            PaymentState::Prepared => "prepared",
            PaymentState::Authorized => "authorized",
            PaymentState::Submitted => "submitted",
            PaymentState::Confirmed => "confirmed",
            PaymentState::Failed => "failed",
            PaymentState::OutcomeUnknown => "outcome_unknown",
            PaymentState::Simulated => "simulated",
        }
    }
}

/// A provider's answer for one operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOutcome {
    pub state: PaymentState,
    pub provider_operation: String,
    /// Whether the outcome came from a simulation rather than a payment provider.
    pub simulated: bool,
}

/// A payment provider, idempotent on the provider operation reference.
pub trait PaymentAdapter: Send + Sync {
    fn submit(
        &self,
        provider_operation: &str,
        total_minor: i64,
        currency: &str,
    ) -> impl Future<Output = Result<ProviderOutcome, String>> + Send;

    fn lookup(
        &self,
        provider_operation: &str,
    ) -> impl Future<Output = Result<Option<ProviderOutcome>, String>> + Send;
}

/// A provider that records operations and moves no money.
#[derive(Debug, Clone)]
pub struct SimulatedProvider {
    pool: SqlitePool,
}

impl SimulatedProvider {
    /// The provider's ledger lives in its own table of the merchant database.
    pub fn new(pool: SqlitePool) -> Self {
        SimulatedProvider { pool }
    }

    /// How many operations the provider holds under `provider_operation` (0 or 1).
    pub async fn operations(&self, provider_operation: &str) -> Result<i64, String> {
        sqlx::query("SELECT COUNT(*) AS n FROM simulated_provider WHERE provider_operation = ?")
            .bind(provider_operation)
            .fetch_one(&self.pool)
            .await
            .map(|r| r.get("n"))
            .map_err(|e| e.to_string())
    }
}

fn outcome(provider_operation: &str) -> ProviderOutcome {
    ProviderOutcome {
        state: PaymentState::Simulated,
        provider_operation: provider_operation.to_owned(),
        simulated: true,
    }
}

impl PaymentAdapter for SimulatedProvider {
    fn submit(
        &self,
        provider_operation: &str,
        total_minor: i64,
        currency: &str,
    ) -> impl Future<Output = Result<ProviderOutcome, String>> + Send {
        let op = provider_operation.to_owned();
        let currency = currency.to_owned();
        async move {
            sqlx::query(
                "INSERT OR IGNORE INTO simulated_provider (provider_operation, total_minor, currency, result) VALUES (?, ?, ?, 'simulated')",
            )
            .bind(&op)
            .bind(total_minor)
            .bind(&currency)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            let row = sqlx::query(
                "SELECT total_minor, currency FROM simulated_provider WHERE provider_operation = ?",
            )
            .bind(&op)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
            if row.get::<i64, _>("total_minor") != total_minor
                || row.get::<String, _>("currency") != currency
            {
                return Err("the provider operation reference was used for another payment".into());
            }
            Ok(outcome(&op))
        }
    }

    fn lookup(
        &self,
        provider_operation: &str,
    ) -> impl Future<Output = Result<Option<ProviderOutcome>, String>> + Send {
        let op = provider_operation.to_owned();
        async move {
            self.operations(&op)
                .await
                .map(|n| (n > 0).then(|| outcome(&op)))
        }
    }
}
