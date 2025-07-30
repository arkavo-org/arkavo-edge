use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Sub, SubAssign};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TokenCost {
    cents: u64,
}

impl TokenCost {
    pub const ZERO: Self = Self { cents: 0 };

    pub fn from_cents(cents: u64) -> Self {
        Self { cents }
    }

    pub fn from_dollars(dollars: f64) -> Self {
        Self {
            cents: (dollars * 100.0) as u64,
        }
    }

    pub fn from_tokens(tokens: u32, cost_per_thousand: TokenCost) -> Self {
        let total_cents = (tokens as u64 * cost_per_thousand.cents) / 1000;
        Self { cents: total_cents }
    }

    pub fn as_cents(&self) -> u64 {
        self.cents
    }

    pub fn as_dollars(&self) -> f64 {
        self.cents as f64 / 100.0
    }

    pub fn is_zero(&self) -> bool {
        self.cents == 0
    }

    pub fn saturating_mul(&self, factor: u64) -> Self {
        Self {
            cents: self.cents.saturating_mul(factor),
        }
    }

    pub fn checked_add(&self, other: Self) -> Option<Self> {
        self.cents
            .checked_add(other.cents)
            .map(|cents| Self { cents })
    }

    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        self.cents
            .checked_sub(other.cents)
            .map(|cents| Self { cents })
    }
}

impl fmt::Display for TokenCost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${:.2}", self.as_dollars())
    }
}

impl Add for TokenCost {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            cents: self.cents + rhs.cents,
        }
    }
}

impl AddAssign for TokenCost {
    fn add_assign(&mut self, rhs: Self) {
        self.cents += rhs.cents;
    }
}

impl Sub for TokenCost {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            cents: self.cents.saturating_sub(rhs.cents),
        }
    }
}

impl SubAssign for TokenCost {
    fn sub_assign(&mut self, rhs: Self) {
        self.cents = self.cents.saturating_sub(rhs.cents);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }

    pub fn total_tokens(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }

    pub fn calculate_cost(
        &self,
        input_cost_per_thousand: TokenCost,
        output_cost_per_thousand: TokenCost,
    ) -> TokenCost {
        let input_cost = TokenCost::from_tokens(self.input_tokens, input_cost_per_thousand);
        let output_cost = TokenCost::from_tokens(self.output_tokens, output_cost_per_thousand);
        input_cost + output_cost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_cost_from_cents() {
        let cost = TokenCost::from_cents(150);
        assert_eq!(cost.as_cents(), 150);
        assert_eq!(cost.as_dollars(), 1.50);
    }

    #[test]
    fn test_token_cost_from_dollars() {
        let cost = TokenCost::from_dollars(2.99);
        assert_eq!(cost.as_cents(), 299);
    }

    #[test]
    fn test_token_cost_from_tokens() {
        let cost_per_thousand = TokenCost::from_cents(30); // $0.30 per 1K tokens
        let cost = TokenCost::from_tokens(1500, cost_per_thousand);
        assert_eq!(cost.as_cents(), 45); // 1.5 * 30 = 45 cents
    }

    #[test]
    fn test_token_cost_arithmetic() {
        let cost1 = TokenCost::from_cents(100);
        let cost2 = TokenCost::from_cents(50);

        assert_eq!((cost1 + cost2).as_cents(), 150);
        assert_eq!((cost1 - cost2).as_cents(), 50);

        let mut cost3 = cost1;
        cost3 += cost2;
        assert_eq!(cost3.as_cents(), 150);
    }

    #[test]
    fn test_token_usage_cost_calculation() {
        let usage = TokenUsage::new(1000, 500);
        let input_cost = TokenCost::from_cents(30); // $0.30 per 1K input tokens
        let output_cost = TokenCost::from_cents(60); // $0.60 per 1K output tokens

        let total_cost = usage.calculate_cost(input_cost, output_cost);
        assert_eq!(total_cost.as_cents(), 60); // 30 + 30 = 60 cents
    }
}
