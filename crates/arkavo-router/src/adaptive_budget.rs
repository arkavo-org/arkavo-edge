use crate::classifier::ProblemComplexity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStats {
    pub total_instances: usize,
    pub total_cost: f64,
    pub average_cost: f64,
    pub simple_count: usize,
    pub medium_count: usize,
    pub complex_count: usize,
    pub local_count: usize,
    pub flash_count: usize,
    pub pro_count: usize,
}

impl BudgetStats {
    pub fn tier_distribution(&self) -> (f64, f64, f64) {
        let total = self.total_instances as f64;
        if total == 0.0 {
            return (0.0, 0.0, 0.0);
        }
        (
            self.simple_count as f64 / total,
            self.medium_count as f64 / total,
            self.complex_count as f64 / total,
        )
    }

    pub fn model_distribution(&self) -> (f64, f64, f64) {
        let total = self.total_instances as f64;
        if total == 0.0 {
            return (0.0, 0.0, 0.0);
        }
        (
            self.local_count as f64 / total,
            self.flash_count as f64 / total,
            self.pro_count as f64 / total,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TierThreshold {
    Standard,
    IncreaseLocal,
    DecreaseLocal,
}

pub struct AdaptiveBudgetManager {
    target_cost_per_instance: f64,
    total_cost: f64,
    total_instances: usize,
    simple_count: usize,
    medium_count: usize,
    complex_count: usize,
    local_count: usize,
    flash_count: usize,
    pro_count: usize,
    alpha: f64,
    running_avg: f64,
}

impl AdaptiveBudgetManager {
    pub fn new(target_cost_per_instance: f64) -> Self {
        Self {
            target_cost_per_instance,
            total_cost: 0.0,
            total_instances: 0,
            simple_count: 0,
            medium_count: 0,
            complex_count: 0,
            local_count: 0,
            flash_count: 0,
            pro_count: 0,
            alpha: 0.2,
            running_avg: 0.0,
        }
    }

    pub fn from_env() -> Self {
        let target = std::env::var("ARKAVO_COST_TARGET")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0003);
        Self::new(target)
    }

    pub fn record_instance(&mut self, cost: f64, complexity: ProblemComplexity, used_local: bool) {
        self.total_cost += cost;
        self.total_instances += 1;

        self.running_avg = if self.total_instances == 1 {
            cost
        } else {
            self.alpha * cost + (1.0 - self.alpha) * self.running_avg
        };

        match complexity {
            ProblemComplexity::Simple => self.simple_count += 1,
            ProblemComplexity::Medium => self.medium_count += 1,
            ProblemComplexity::Complex => self.complex_count += 1,
        }

        if used_local {
            self.local_count += 1;
        } else if cost < 0.0001 {
            self.flash_count += 1;
        } else {
            self.pro_count += 1;
        }
    }

    pub fn average_cost(&self) -> f64 {
        if self.total_instances == 0 {
            return 0.0;
        }
        self.total_cost / self.total_instances as f64
    }

    pub fn get_tier_threshold(&self) -> TierThreshold {
        if self.total_instances < 5 {
            return TierThreshold::Standard;
        }

        let avg = self.running_avg;

        if avg > self.target_cost_per_instance * 1.2 {
            TierThreshold::IncreaseLocal
        } else if avg < self.target_cost_per_instance * 0.8 {
            TierThreshold::DecreaseLocal
        } else {
            TierThreshold::Standard
        }
    }

    pub fn should_use_local_for_medium(&self) -> bool {
        matches!(self.get_tier_threshold(), TierThreshold::IncreaseLocal)
    }

    pub fn should_use_pro_for_medium(&self) -> bool {
        matches!(self.get_tier_threshold(), TierThreshold::DecreaseLocal)
    }

    pub fn get_stats(&self) -> BudgetStats {
        BudgetStats {
            total_instances: self.total_instances,
            total_cost: self.total_cost,
            average_cost: self.average_cost(),
            simple_count: self.simple_count,
            medium_count: self.medium_count,
            complex_count: self.complex_count,
            local_count: self.local_count,
            flash_count: self.flash_count,
            pro_count: self.pro_count,
        }
    }

    pub fn budget_utilization(&self) -> f64 {
        if self.total_instances == 0 {
            return 0.0;
        }
        self.average_cost() / self.target_cost_per_instance
    }

    pub fn remaining_budget_for(&self, total_target_instances: usize) -> f64 {
        let target_total = self.target_cost_per_instance * total_target_instances as f64;
        target_total - self.total_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_manager_initialization() {
        let manager = AdaptiveBudgetManager::new(0.0003);
        assert_eq!(manager.total_instances, 0);
        assert_eq!(manager.total_cost, 0.0);
        assert_eq!(manager.average_cost(), 0.0);
    }

    #[test]
    fn test_record_instance_local() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        manager.record_instance(0.0, ProblemComplexity::Simple, true);

        assert_eq!(manager.total_instances, 1);
        assert_eq!(manager.simple_count, 1);
        assert_eq!(manager.local_count, 1);
    }

    #[test]
    fn test_average_cost_calculation() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        manager.record_instance(0.0001, ProblemComplexity::Simple, true);
        manager.record_instance(0.0002, ProblemComplexity::Medium, false);
        manager.record_instance(0.0005, ProblemComplexity::Complex, false);

        let avg = manager.average_cost();
        assert!((avg - 0.000267).abs() < 0.000001);
    }

    #[test]
    fn test_tier_threshold_standard() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        for _ in 0..10 {
            manager.record_instance(0.0003, ProblemComplexity::Medium, false);
        }

        assert_eq!(manager.get_tier_threshold(), TierThreshold::Standard);
    }

    #[test]
    fn test_tier_threshold_increase_local() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        for _ in 0..10 {
            manager.record_instance(0.0005, ProblemComplexity::Complex, false);
        }

        assert_eq!(manager.get_tier_threshold(), TierThreshold::IncreaseLocal);
        assert!(manager.should_use_local_for_medium());
    }

    #[test]
    fn test_tier_threshold_decrease_local() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        for _ in 0..10 {
            manager.record_instance(0.0001, ProblemComplexity::Simple, true);
        }

        assert_eq!(manager.get_tier_threshold(), TierThreshold::DecreaseLocal);
        assert!(manager.should_use_pro_for_medium());
    }

    #[test]
    fn test_budget_utilization() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        manager.record_instance(0.0003, ProblemComplexity::Medium, false);

        let utilization = manager.budget_utilization();
        assert!((utilization - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_tier_distribution() {
        let mut manager = AdaptiveBudgetManager::new(0.0003);
        manager.record_instance(0.0, ProblemComplexity::Simple, true);
        manager.record_instance(0.0001, ProblemComplexity::Medium, false);
        manager.record_instance(0.0005, ProblemComplexity::Complex, false);

        let stats = manager.get_stats();
        let (simple, medium, complex) = stats.tier_distribution();

        assert!((simple - 0.333).abs() < 0.01);
        assert!((medium - 0.333).abs() < 0.01);
        assert!((complex - 0.333).abs() < 0.01);
    }
}
