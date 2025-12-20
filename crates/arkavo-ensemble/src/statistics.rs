//! Statistical tests for candidate evaluation
//!
//! Implements Welch's t-test for comparing candidate performance
//! against production policy.

use std::f64::consts::PI;

/// Default significance threshold (alpha = 0.05)
pub const DEFAULT_SIGNIFICANCE_THRESHOLD: f64 = 0.05;

/// Result of a Welch's t-test
#[derive(Debug, Clone)]
pub struct TTestResult {
    /// The t-statistic
    pub t_statistic: f64,
    /// Welch-Satterthwaite degrees of freedom
    pub degrees_of_freedom: f64,
    /// Two-tailed p-value
    pub p_value: f64,
}

impl TTestResult {
    /// Check if the result is statistically significant at the given threshold
    pub fn is_significant(&self, threshold: f64) -> bool {
        self.p_value < threshold
    }

    /// Check if significant at the default threshold (0.05)
    pub fn is_significant_default(&self) -> bool {
        self.is_significant(DEFAULT_SIGNIFICANCE_THRESHOLD)
    }
}

/// Perform Welch's t-test for unequal variances
///
/// Compares two samples with potentially different variances and sizes.
///
/// # Arguments
/// * `mean1`, `var1`, `n1` - Mean, variance, and sample size of first sample
/// * `mean2`, `var2`, `n2` - Mean, variance, and sample size of second sample
///
/// # Returns
/// Test result with t-statistic, degrees of freedom, and p-value
pub fn welch_t_test(mean1: f64, var1: f64, n1: u64, mean2: f64, var2: f64, n2: u64) -> TTestResult {
    // Ensure we have enough samples
    if n1 < 2 || n2 < 2 {
        return TTestResult {
            t_statistic: 0.0,
            degrees_of_freedom: 0.0,
            p_value: 1.0, // Not significant
        };
    }

    let n1_f = n1 as f64;
    let n2_f = n2 as f64;

    // Standard errors squared
    let s1 = var1 / n1_f;
    let s2 = var2 / n2_f;

    // Pooled standard error
    let se = (s1 + s2).sqrt();

    if se < f64::EPSILON {
        // No variance, can't compute t-statistic
        return TTestResult {
            t_statistic: 0.0,
            degrees_of_freedom: 0.0,
            p_value: 1.0,
        };
    }

    // T-statistic
    let t = (mean1 - mean2) / se;

    // Welch-Satterthwaite degrees of freedom
    let df_num = (s1 + s2).powi(2);
    let df_den = (s1.powi(2) / (n1_f - 1.0)) + (s2.powi(2) / (n2_f - 1.0));

    let df = if df_den > f64::EPSILON {
        df_num / df_den
    } else {
        (n1_f - 1.0).min(n2_f - 1.0)
    };

    // Compute two-tailed p-value
    let p_value = student_t_cdf_two_tailed(t.abs(), df);

    TTestResult {
        t_statistic: t,
        degrees_of_freedom: df,
        p_value,
    }
}

/// Compute two-tailed p-value from Student's t distribution
///
/// Uses the regularized incomplete beta function approximation.
fn student_t_cdf_two_tailed(t: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return 1.0;
    }

    // Use the relationship between t-distribution and beta distribution
    // P(|T| > t) = I_{df/(df + t^2)}(df/2, 1/2)
    let x = df / (df + t * t);

    // For the two-tailed test, p-value = 2 * (1 - CDF(|t|))
    // Using the regularized incomplete beta function
    regularized_incomplete_beta(x, df / 2.0, 0.5)
}

/// Regularized incomplete beta function I_x(a, b)
///
/// Uses the continued fraction approximation.
fn regularized_incomplete_beta(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    // Use continued fraction for better convergence
    let bt = if x == 0.0 || x == 1.0 {
        0.0
    } else {
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp()
    };

    // Use symmetry for better convergence
    if x < (a + 1.0) / (a + b + 2.0) {
        bt * beta_continued_fraction(x, a, b) / a
    } else {
        1.0 - bt * beta_continued_fraction(1.0 - x, b, a) / b
    }
}

/// Continued fraction for incomplete beta function
fn beta_continued_fraction(x: f64, a: f64, b: f64) -> f64 {
    const MAX_ITER: usize = 100;
    const EPSILON: f64 = 1e-10;
    const TINY: f64 = 1e-30;

    let apb = a + b;
    let ap1 = a + 1.0;
    let am1 = a - 1.0;

    let mut c = 1.0;
    let mut d = 1.0 - apb * x / ap1;

    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=MAX_ITER {
        let m_f = m as f64;
        let m2 = 2.0 * m_f;

        // Even step
        let aa = m_f * (b - m_f) * x / ((am1 + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;

        // Odd step
        let aa = -(a + m_f) * (apb + m_f) * x / ((a + m2) * (ap1 + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;

        if (delta - 1.0).abs() < EPSILON {
            break;
        }
    }

    h
}

/// Natural log of gamma function using Lanczos approximation
#[allow(clippy::excessive_precision)]
fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_93,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_9,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_571_6e-6,
        1.505_632_735_149_311_6e-7,
    ];

    if x < 0.5 {
        // Reflection formula
        let sin_pi_x = (PI * x).sin();
        if sin_pi_x.abs() < f64::EPSILON {
            return f64::INFINITY;
        }
        PI.ln() - sin_pi_x.ln() - ln_gamma(1.0 - x)
    } else {
        let x = x - 1.0;
        let mut ag = COEFFICIENTS[0];
        for (i, &coef) in COEFFICIENTS.iter().enumerate().skip(1) {
            ag += coef / (x + i as f64);
        }

        let tmp = x + G + 0.5;
        0.5 * (2.0 * PI).ln() + (x + 0.5) * tmp.ln() - tmp + ag.ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_welch_t_test_equal_samples() {
        // Two identical samples should have p-value = 1
        let result = welch_t_test(5.0, 1.0, 30, 5.0, 1.0, 30);
        assert!((result.t_statistic).abs() < 0.001);
        assert!(result.p_value > 0.99);
    }

    #[test]
    fn test_welch_t_test_different_samples() {
        // Very different samples should have low p-value
        let result = welch_t_test(0.0, 1.0, 100, 5.0, 1.0, 100);
        assert!(result.p_value < 0.001);
        assert!(result.is_significant_default());
    }

    #[test]
    fn test_welch_t_test_insufficient_samples() {
        // With n < 2, should return not significant
        let result = welch_t_test(0.0, 1.0, 1, 5.0, 1.0, 1);
        assert_eq!(result.p_value, 1.0);
        assert!(!result.is_significant_default());
    }

    #[test]
    fn test_welch_t_test_known_value() {
        // Test against known statistical values
        // Sample 1: mean=10, var=4, n=10
        // Sample 2: mean=12, var=5, n=12
        let result = welch_t_test(10.0, 4.0, 10, 12.0, 5.0, 12);

        // t-statistic should be approximately -2.16
        assert!((result.t_statistic + 2.16).abs() < 0.1);

        // df should be approximately 19.5
        assert!((result.degrees_of_freedom - 19.5).abs() < 1.0);

        // p-value should be around 0.04
        assert!(result.p_value < 0.1);
    }

    #[test]
    fn test_significance_threshold() {
        let result = TTestResult {
            t_statistic: 2.0,
            degrees_of_freedom: 20.0,
            p_value: 0.03,
        };

        assert!(result.is_significant(0.05));
        assert!(!result.is_significant(0.01));
        assert!(result.is_significant_default());
    }

    #[test]
    fn test_ln_gamma_known_values() {
        // ln(Gamma(1)) = 0
        assert!((ln_gamma(1.0)).abs() < 0.001);

        // ln(Gamma(2)) = ln(1!) = 0
        assert!((ln_gamma(2.0)).abs() < 0.001);

        // ln(Gamma(5)) = ln(4!) = ln(24) ≈ 3.178
        assert!((ln_gamma(5.0) - 3.178).abs() < 0.01);
    }
}
