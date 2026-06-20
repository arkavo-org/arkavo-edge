pub mod api;
pub mod cloud_policy;
pub mod compute_budget;
pub mod config;
pub mod cost;
pub mod middleware;
pub mod policy;
pub mod provider_costs;
pub mod tracker;

pub use cloud_policy::{
    CloudPolicy, CloudSpendDecision, CloudSpendReason, CloudSpendRequest, DenyReason, SpendCaps,
    authorize_cloud_spend,
};
pub use compute_budget::{
    AgentAllocationSummary, AgentComputeBudget, BudgetAllocation, BudgetPolicy,
    ComputeBudgetSnapshot, SharedComputeBudget, UrgencyLevel, new_shared_compute_budget,
};
pub use config::{BudgetConfig, BudgetLimits, BudgetThresholds};
pub use cost::TokenCost;
pub use middleware::{BudgetMiddleware, BudgetProviderBuilder};
pub use policy::{ModelSelectionPolicy, SelectionCriteria};
pub use provider_costs::PricingEntry;
pub use tracker::{
    ArchitectCostMetadata, ArchitectSavingsReport, ArchitectUsageSummary, BudgetStatusWithLimits,
    BudgetTracker,
};

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct BudgetManager {
    tracker: Arc<BudgetTracker>,
    config: Arc<RwLock<BudgetConfig>>,
    policy: Arc<ModelSelectionPolicy>,
}

impl BudgetManager {
    pub async fn new(config: BudgetConfig) -> Result<Self> {
        let tracker = Arc::new(BudgetTracker::new(config.clone()).await?);
        let policy = Arc::new(ModelSelectionPolicy::new());

        Ok(Self {
            tracker,
            config: Arc::new(RwLock::new(config)),
            policy,
        })
    }

    pub fn tracker(&self) -> Arc<BudgetTracker> {
        Arc::clone(&self.tracker)
    }

    pub fn policy(&self) -> Arc<ModelSelectionPolicy> {
        Arc::clone(&self.policy)
    }

    pub async fn update_config(&self, config: BudgetConfig) -> Result<()> {
        let mut cfg = self.config.write().await;
        *cfg = config;
        Ok(())
    }

    pub async fn get_config(&self) -> BudgetConfig {
        self.config.read().await.clone()
    }
}
