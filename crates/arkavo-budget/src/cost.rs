use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};

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

    /// Cost of `tokens` priced **per 1K tokens**. Prefer
    /// [`Self::from_tokens_per_million`] for cloud rates: `ProviderPricing`
    /// stores cents-per-MTok, and feeding a per-MTok rate here (or vice versa)
    /// is a silent 1000x error. This per-1K form remains only for the legacy
    /// `TokenUsage::calculate_cost` path.
    pub fn from_tokens(tokens: u32, cost_per_thousand: TokenCost) -> Self {
        let total_cents = (tokens as u64 * cost_per_thousand.cents) / 1000;
        Self { cents: total_cents }
    }

    /// Cost of `tokens` priced at `cost_per_million` (cents per 1M tokens).
    ///
    /// Modern cloud rates are quoted per-MTok and are routinely sub-cent per
    /// 1K (GLM-5.2 input is $1.40/MTok = 0.14c/1K), which floors to zero in the
    /// per-1K integer unit. Pricing the rate per-MTok keeps it representable.
    pub fn from_tokens_per_million(tokens: u32, cost_per_million: TokenCost) -> Self {
        let total_cents = (tokens as u64 * cost_per_million.cents) / 1_000_000;
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

impl Mul<u64> for TokenCost {
    type Output = Self;

    fn mul(self, rhs: u64) -> Self::Output {
        Self {
            cents: self.cents.saturating_mul(rhs),
        }
    }
}

impl Div<u64> for TokenCost {
    type Output = Self;

    fn div(self, rhs: u64) -> Self::Output {
        Self {
            cents: self.cents / rhs,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens read from the prompt cache (billed at discounted rate)
    #[serde(default)]
    pub cached_input_tokens: u32,
    /// Tokens written into the prompt cache. Disjoint from `input_tokens` and
    /// `cached_input_tokens`, and billed at the total per-MTok rate for
    /// cache-write tokens — not at a surcharge over the input rate.
    #[serde(default)]
    pub cache_write_tokens: u32,
    /// Hidden chain-of-thought tokens (Gemini 3.5's `thoughtsTokenCount`).
    /// Billed at the output rate by Gemini but invisible to the response
    /// stream — tracked separately so latency analysis and per-tier
    /// thinking-budget reporting can attribute spend correctly.
    #[serde(default)]
    pub thinking_tokens: u32,
}

impl TokenUsage {
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            thinking_tokens: 0,
        }
    }

    pub fn with_cache(
        input_tokens: u32,
        output_tokens: u32,
        cached_input_tokens: u32,
        cache_write_tokens: u32,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens,
            cache_write_tokens,
            thinking_tokens: 0,
        }
    }

    /// Build a TokenUsage from real provider-reported token counts.
    /// Pass `thinking_tokens=None` for providers that don't separate
    /// chain-of-thought tokens from regular output.
    pub fn with_thinking(input_tokens: u32, output_tokens: u32, thinking_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cached_input_tokens: 0,
            cache_write_tokens: 0,
            thinking_tokens,
        }
    }

    pub fn total_tokens(&self) -> u32 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cached_input_tokens)
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.thinking_tokens)
    }

    pub fn calculate_cost(
        &self,
        input_cost_per_thousand: TokenCost,
        output_cost_per_thousand: TokenCost,
    ) -> TokenCost {
        let input_cost = TokenCost::from_tokens(self.input_tokens, input_cost_per_thousand);
        // Thinking tokens bill at the output rate (Gemini pricing rule).
        let billable_output = self.output_tokens.saturating_add(self.thinking_tokens);
        let output_cost = TokenCost::from_tokens(billable_output, output_cost_per_thousand);
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
    fn test_token_cost_from_tokens_per_million() {
        // Per-MTok is the unit modern cloud rates are quoted in, and the only
        // resolution that survives sub-cent-per-1K rates. GLM-5.2 input is
        // $1.40/MTok = 140 cents/MTok; a 200K-token prompt costs $0.28 = 28c.
        let rate_per_mtok = TokenCost::from_cents(140);
        let cost = TokenCost::from_tokens_per_million(200_000, rate_per_mtok);
        assert_eq!(cost.as_cents(), 28);
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

        // Test multiplication and division
        assert_eq!((cost1 * 2).as_cents(), 200);
        assert_eq!((cost1 / 2).as_cents(), 50);
    }

    #[test]
    fn test_token_usage_cost_calculation() {
        let usage = TokenUsage::new(1000, 500);
        let input_cost = TokenCost::from_cents(30); // $0.30 per 1K input tokens
        let output_cost = TokenCost::from_cents(60); // $0.60 per 1K output tokens

        let total_cost = usage.calculate_cost(input_cost, output_cost);
        assert_eq!(total_cost.as_cents(), 60); // 30 + 30 = 60 cents
    }

    #[test]
    fn test_token_usage_with_cache() {
        let usage = TokenUsage::with_cache(500, 200, 800, 100);
        assert_eq!(usage.input_tokens, 500);
        assert_eq!(usage.cached_input_tokens, 800);
        assert_eq!(usage.cache_write_tokens, 100);
        assert_eq!(usage.total_tokens(), 1600);
    }

    #[test]
    fn test_token_usage_new_has_zero_cache() {
        let usage = TokenUsage::new(1000, 500);
        assert_eq!(usage.cached_input_tokens, 0);
        assert_eq!(usage.cache_write_tokens, 0);
    }
}
