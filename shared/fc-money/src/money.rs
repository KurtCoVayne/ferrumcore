//! `Money` — the currency-scaled value type. Wraps the exact-decimal core, normalizing to the currency's minor-unit
//! scale on construction and after every operation (validated against the real Fineract `Money`,
//! `plan/harness/golden/money_corpus.tsv`, 3654/3654). The only monetary arithmetic type in the system (ADR-0005).
use crate::decimal;
use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;

/// A monetary amount fixed to `scale` minor-unit digits, rounded with `mode`.
#[derive(Clone, Debug)]
pub struct Money {
    amount: BigDecimal,
    scale: i64,
    mode: RoundingMode,
}

impl Money {
    /// Construct from a raw decimal, normalizing to `scale` (currency minor units) with `mode`.
    #[must_use]
    pub fn of(scale: i64, mode: RoundingMode, raw: &BigDecimal) -> Self {
        Self {
            amount: decimal::set_scale(raw, scale, mode),
            scale,
            mode,
        }
    }

    /// `self + other` (operands already normalized), result re-normalized to `scale`.
    #[must_use]
    pub fn plus(&self, other: &Money) -> Self {
        self.normalized(decimal::add(&self.amount, &other.amount))
    }
    /// `self - other`, result re-normalized to `scale`.
    #[must_use]
    pub fn minus(&self, other: &Money) -> Self {
        self.normalized(decimal::sub(&self.amount, &other.amount))
    }
    /// `self * factor` (raw factor), result re-normalized to `scale`.
    #[must_use]
    pub fn multiplied_by(&self, factor: &BigDecimal) -> Self {
        self.normalized(decimal::mul(&self.amount, factor))
    }
    /// `self / divisor`, correctly rounded to `scale`.
    #[must_use]
    pub fn divided_by(&self, divisor: &BigDecimal) -> Self {
        Self {
            amount: decimal::div(&self.amount, divisor, self.scale, self.mode),
            scale: self.scale,
            mode: self.mode,
        }
    }

    /// `self / divisor` rounded to `scale` with a caller-chosen `mode` (Java `Money.dividedBy(divisor, MathContext)`
    /// with an explicit `RoundingMode`). Used by the advanced payment-allocation IN_ADVANCE even-split's
    /// `RoundingMode.DOWN` safety branch, which overrides the default rounding to guarantee the split cannot exceed the
    /// dividend.
    #[must_use]
    pub fn divided_by_with_mode(&self, divisor: &BigDecimal, mode: RoundingMode) -> Self {
        Self {
            amount: decimal::div(&self.amount, divisor, self.scale, mode),
            scale: self.scale,
            mode: self.mode,
        }
    }

    /// A zero amount at `scale`/`mode` (Java `Money.zero(currency)`).
    #[must_use]
    pub fn zero(scale: i64, mode: RoundingMode) -> Self {
        Self::of(scale, mode, &BigDecimal::from(0))
    }

    /// The underlying normalized amount.
    #[must_use]
    pub fn amount(&self) -> &BigDecimal {
        &self.amount
    }

    /// `this == 0` (Java `Money.isZero`).
    #[must_use]
    pub fn is_zero(&self) -> bool {
        decimal::compare(&self.amount, &BigDecimal::from(0)) == 0
    }
    /// `this > 0` (Java `Money.isGreaterThanZero`).
    #[must_use]
    pub fn is_greater_than_zero(&self) -> bool {
        decimal::compare(&self.amount, &BigDecimal::from(0)) > 0
    }
    /// `this < 0` (Java `Money.isLessThanZero`).
    #[must_use]
    pub fn is_less_than_zero(&self) -> bool {
        decimal::compare(&self.amount, &BigDecimal::from(0)) < 0
    }
    /// `this > other` (Java `Money.isGreaterThan`).
    #[must_use]
    pub fn is_greater_than(&self, other: &Money) -> bool {
        decimal::compare(&self.amount, &other.amount) > 0
    }
    /// `this >= other` (Java `Money.isGreaterThanOrEqualTo`).
    #[must_use]
    pub fn is_greater_than_or_equal_to(&self, other: &Money) -> bool {
        decimal::compare(&self.amount, &other.amount) >= 0
    }

    /// The amount as a Java-`toPlainString`-equivalent (no sci-notation; trailing zeros to scale).
    #[must_use]
    pub fn to_plain_string(&self) -> String {
        decimal::to_plain_string(&self.amount)
    }

    fn normalized(&self, raw: BigDecimal) -> Self {
        Self {
            amount: decimal::set_scale(&raw, self.scale, self.mode),
            scale: self.scale,
            mode: self.mode,
        }
    }
}
