use std::fmt;
use std::ops::{Add, Mul, Neg, Sub};

/// Monetary amount stored as integer units at 10^8 scale.
/// 1 dollar = 100_000_000 internal units.
/// Max representable: ~$92.2 billion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money(pub i64);

const SCALE: i64 = 100_000_000; // 10^8
const CENTS_SCALE: i64 = 1_000_000; // 10^6 — rounds to 2 decimal places

impl Money {
    /// Constructs a `Money` value from a floating-point dollar amount.
    /// For parsing user input only. Rounds at the 8th decimal place.
    pub fn from_dollars(dollars: f64) -> Self {
        Self((dollars * SCALE as f64).round() as i64)
    }

    /// Returns the value rounded to 2 decimal places (cents) as an integer.
    /// e.g., `Money(123456789012).cents_rounded() == 123457`
    pub fn cents_rounded(&self) -> i64 {
        let base = self.0 / CENTS_SCALE;
        let remainder = self.0.unsigned_abs() % (CENTS_SCALE as u64);
        if remainder >= (CENTS_SCALE as u64 / 2) {
            if self.0 >= 0 { base + 1 } else { base - 1 }
        } else {
            base
        }
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }
}

fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(*c);
    }
    result
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cents = self.cents_rounded();
        let sign = if cents < 0 { "-" } else { "" };
        let abs_cents = cents.unsigned_abs();
        let dollars = abs_cents / 100;
        let fraction = abs_cents % 100;
        write!(f, "{}{}.{:02}", sign, format_with_commas(dollars), fraction)
    }
}

impl Add for Money {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Money {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul<i64> for Money {
    type Output = Self;
    fn mul(self, rhs: i64) -> Self {
        Self(self.0 * rhs)
    }
}

impl Neg for Money {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_thousands_separator() {
        assert_eq!(Money::from_dollars(1234.56).to_string(), "1,234.56");
    }

    #[test]
    fn display_zero() {
        assert_eq!(Money::from_dollars(0.0).to_string(), "0.00");
    }

    #[test]
    fn display_negative() {
        assert_eq!(Money::from_dollars(-42.50).to_string(), "-42.50");
    }

    #[test]
    fn display_large() {
        // spec example: Money(123456789012) displays as "1,234.57"
        assert_eq!(Money(123_456_789_012).to_string(), "1,234.57");
    }

    #[test]
    fn is_zero_true() {
        assert!(Money::from_dollars(0.0).is_zero());
    }

    #[test]
    fn is_zero_false() {
        assert!(!Money::from_dollars(1.0).is_zero());
    }

    #[test]
    fn arithmetic_add() {
        assert_eq!(Money(100) + Money(200), Money(300));
    }

    #[test]
    fn arithmetic_sub() {
        assert_eq!(Money(500) - Money(300), Money(200));
    }

    #[test]
    fn arithmetic_mul() {
        assert_eq!(Money(100) * 3, Money(300));
    }

    #[test]
    fn arithmetic_neg() {
        assert_eq!(-Money(100), Money(-100));
    }

    #[test]
    fn abs_negative() {
        assert_eq!(
            Money::from_dollars(-100.0).abs(),
            Money::from_dollars(100.0)
        );
    }

    #[test]
    fn abs_positive_unchanged() {
        assert_eq!(Money::from_dollars(50.0).abs(), Money::from_dollars(50.0));
    }

    #[test]
    fn cents_rounded_rounding_up() {
        // 1234.56789012 → rounds to 123457 cents
        assert_eq!(Money(123_456_789_012).cents_rounded(), 123_457);
    }

    #[test]
    fn cents_rounded_exact() {
        // 1000.00 exactly
        let m = Money::from_dollars(1000.0);
        assert_eq!(m.cents_rounded(), 100_000);
    }

    #[test]
    fn from_dollars_round_trip_display() {
        // $99.99 should display correctly
        assert_eq!(Money::from_dollars(99.99).to_string(), "99.99");
    }
}
