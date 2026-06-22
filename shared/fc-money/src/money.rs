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
