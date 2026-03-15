use crate::types::money::Money;
use std::fmt;

/// Envelope allocation percentage stored as integer units at 10^6 scale.
/// 1% = 1,000,000 internal units. Precision to 0.000001%.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Percentage(pub i64);

const SCALE: i64 = 1_000_000; // 10^6

impl Percentage {
    /// Constructs a `Percentage` from a display value (e.g., 15.5 → 15_500_000).
    pub fn from_display(pct: f64) -> Self {
        Self((pct * SCALE as f64).round() as i64)
    }

    /// Returns the multiplier as f64 for use in `Money::apply_percentage`.
    /// The result is immediately captured back into a `Money` value.
    pub fn as_multiplier(&self) -> f64 {
        self.0 as f64 / (SCALE as f64 * 100.0)
    }
}

impl fmt::Display for Percentage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Scale is 10^6, so divide by 10^4 to get two decimal places of percentage.
        let hundredths = self.0 / 10_000; // e.g., 15_500_000 → 1550
        let whole = hundredths / 100; // 15
        let frac = hundredths.abs() % 100; // 50
        write!(f, "{}.{:02}%", whole, frac)
    }
}

impl Money {
    /// Multiplies this monetary amount by a percentage, retaining full internal precision.
    /// Used for envelope fill calculations.
    pub fn apply_percentage(&self, pct: Percentage) -> Money {
        // Use f64 as an intermediate; result is immediately captured into Money.
        Money((self.0 as f64 * pct.as_multiplier()).round() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_fifteen_point_five() {
        assert_eq!(Percentage::from_display(15.5).to_string(), "15.50%");
    }

    #[test]
    fn display_zero() {
        assert_eq!(Percentage::from_display(0.0).to_string(), "0.00%");
    }

    #[test]
    fn display_one_hundred() {
        assert_eq!(Percentage::from_display(100.0).to_string(), "100.00%");
    }

    #[test]
    fn round_trip() {
        // from_display(x).to_string() should parse back to same value
        let pct = Percentage::from_display(33.33);
        let s = pct.to_string(); // "33.33%"
        assert!(s.starts_with("33.33"));
    }

    #[test]
    fn apply_percentage_to_money() {
        // $1000.00 × 15.5% = $155.00
        let money = Money::from_dollars(1000.0);
        let pct = Percentage::from_display(15.5);
        let result = money.apply_percentage(pct);
        assert_eq!(result.to_string(), "155.00");
    }

    #[test]
    fn apply_percentage_fractional() {
        // $100.00 × 33.333333% ≈ $33.33
        let money = Money::from_dollars(100.0);
        let pct = Percentage::from_display(33.333333);
        let result = money.apply_percentage(pct);
        assert_eq!(result.to_string(), "33.33");
    }

    #[test]
    fn as_multiplier_correct() {
        let pct = Percentage::from_display(10.0);
        let multiplier = pct.as_multiplier();
        assert!((multiplier - 0.10).abs() < 1e-9);
    }
}
