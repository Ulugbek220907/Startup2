//! Money in Uzbek soʻm.
//!
//! Amounts are stored as integer **tiyin** (1 soʻm = 100 tiyin) in an `i64`
//! and never touch floating point. Tiyin coins are no longer used in daily
//! life, but payment providers disagree on units (Payme's API takes tiyin,
//! Click's takes soʻm), so the conversions are explicit and tested here
//! instead of being scattered `* 100`s.

use serde::{Deserialize, Serialize};

pub const TIYIN_PER_SUM: i64 = 100;

/// An amount of money in tiyin.
///
/// `#[serde(transparent)]` makes it serialise as a bare integer, so JSON
/// carries `13000000`, not `{"0":13000000}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tiyin(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoneyError {
    Overflow,
    Negative,
    /// The amount has a fractional soʻm part and cannot be sent to a
    /// provider that only accepts whole soʻm.
    NotWholeSum,
}

impl std::fmt::Display for MoneyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            MoneyError::Overflow => "amount is too large",
            MoneyError::Negative => "amount is negative",
            MoneyError::NotWholeSum => "amount is not a whole number of soʻm",
        })
    }
}

impl std::error::Error for MoneyError {}

impl Tiyin {
    pub const ZERO: Tiyin = Tiyin(0);

    /// Whole soʻm -> tiyin.
    pub fn from_sum(sum: i64) -> Result<Tiyin, MoneyError> {
        sum.checked_mul(TIYIN_PER_SUM).map(Tiyin).ok_or(MoneyError::Overflow)
    }

    /// Tiyin -> whole soʻm, failing if a fractional part would be lost.
    pub fn to_sum_exact(self) -> Result<i64, MoneyError> {
        if self.0 % TIYIN_PER_SUM != 0 {
            return Err(MoneyError::NotWholeSum);
        }
        Ok(self.0 / TIYIN_PER_SUM)
    }

    /// Tiyin -> soʻm, rounding towards negative infinity.
    pub fn to_sum_floor(self) -> i64 {
        self.0.div_euclid(TIYIN_PER_SUM)
    }

    /// Round down to a whole soʻm (used for discounts, so cash totals never
    /// contain tiyin).
    pub fn floor_to_sum(self) -> Tiyin {
        Tiyin(self.to_sum_floor() * TIYIN_PER_SUM)
    }

    pub fn checked_add(self, o: Tiyin) -> Result<Tiyin, MoneyError> {
        self.0.checked_add(o.0).map(Tiyin).ok_or(MoneyError::Overflow)
    }

    pub fn checked_sub(self, o: Tiyin) -> Result<Tiyin, MoneyError> {
        self.0.checked_sub(o.0).map(Tiyin).ok_or(MoneyError::Overflow)
    }

    pub fn checked_mul(self, k: i64) -> Result<Tiyin, MoneyError> {
        self.0.checked_mul(k).map(Tiyin).ok_or(MoneyError::Overflow)
    }

    /// `self * bp / 10_000` rounded down, where `bp` is basis points
    /// (100 bp = 1 %). Uses i128 internally so it cannot overflow.
    pub fn basis_points(self, bp: u32) -> Tiyin {
        Tiyin(((self.0 as i128 * bp as i128).div_euclid(10_000)) as i64)
    }

    pub fn require_non_negative(self) -> Result<Tiyin, MoneyError> {
        if self.0 < 0 {
            Err(MoneyError::Negative)
        } else {
            Ok(self)
        }
    }

    /// `"130 000"`: whole soʻm with a space as thousands separator, the
    /// usual way prices are written in Uzbekistan. Tiyin are shown only when
    /// present (`"130 000,50"`).
    pub fn format_sum(self) -> String {
        let neg = self.0 < 0;
        let abs = self.0.unsigned_abs();
        let (sum, tiyin) = (abs / TIYIN_PER_SUM as u64, abs % TIYIN_PER_SUM as u64);
        let digits = sum.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 4);
        if neg {
            out.push('-');
        }
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                // Narrow no-break space keeps the number on one line.
                out.push('\u{202F}');
            }
            out.push(ch);
        }
        if tiyin != 0 {
            out.push_str(&format!(",{tiyin:02}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn formats_like_local_price_tags() {
        assert_eq!(Tiyin::from_sum(130_000).unwrap().format_sum(), "130\u{202F}000");
        assert_eq!(Tiyin::from_sum(1_250_000).unwrap().format_sum(), "1\u{202F}250\u{202F}000");
        assert_eq!(Tiyin(0).format_sum(), "0");
        assert_eq!(Tiyin(99_950).format_sum(), "999,50");
        assert_eq!(Tiyin(-500_000).format_sum(), "-5\u{202F}000");
    }

    #[test]
    fn provider_unit_conversions() {
        // Payme-style (tiyin) and Click-style (soʻm) views of one amount.
        let price = Tiyin::from_sum(260_000).unwrap();
        assert_eq!(price.0, 26_000_000);
        assert_eq!(price.to_sum_exact(), Ok(260_000));
        assert_eq!(Tiyin(26_000_050).to_sum_exact(), Err(MoneyError::NotWholeSum));
        assert_eq!(Tiyin(26_000_050).to_sum_floor(), 260_000);
        assert_eq!(Tiyin::from_sum(i64::MAX), Err(MoneyError::Overflow));
    }

    #[test]
    fn basis_points_round_down() {
        assert_eq!(Tiyin(10_000).basis_points(500), Tiyin(500)); // 5 %
        assert_eq!(Tiyin(999).basis_points(1), Tiyin(0));
        assert_eq!(Tiyin(i64::MAX).basis_points(10_000), Tiyin(i64::MAX));
    }

    proptest! {
        #[test]
        fn sum_round_trip(sum in -(i64::MAX / 100)..(i64::MAX / 100)) {
            let t = Tiyin::from_sum(sum).unwrap();
            prop_assert_eq!(t.to_sum_exact(), Ok(sum));
            prop_assert_eq!(t.to_sum_floor(), sum);
            prop_assert_eq!(t.floor_to_sum(), t);
        }

        #[test]
        fn floor_never_increases_and_loses_less_than_one_sum(t in any::<i64>().prop_filter("room to floor", |v| *v > i64::MIN + 100)) {
            let f = Tiyin(t).floor_to_sum();
            prop_assert!(f.0 <= t);
            prop_assert!(t - f.0 < TIYIN_PER_SUM);
            prop_assert_eq!(f.0 % TIYIN_PER_SUM, 0);
        }

        #[test]
        fn basis_points_bounded(t in 0..i64::MAX, bp in 0u32..=10_000) {
            let part = Tiyin(t).basis_points(bp);
            prop_assert!(part.0 >= 0 && part.0 <= t);
        }

        #[test]
        fn format_parses_back(sum in 0i64..10_000_000_000) {
            let s = Tiyin::from_sum(sum).unwrap().format_sum();
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            prop_assert_eq!(digits.parse::<i64>().unwrap(), sum);
        }
    }
}
