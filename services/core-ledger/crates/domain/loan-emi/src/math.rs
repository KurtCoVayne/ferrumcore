//! Faithful Java `MathContext` arithmetic over `BigDecimal` — significant-figure rounding that matches
//! `java.math.BigDecimal`'s `add`/`multiply`/`subtract`/`divide(…, mc)` exactly, **including** the no-padding rule
//! (a value with fewer than `precision` significant figures is returned unchanged) and `divide`'s terminating-quotient
//! "preferred scale" (e.g. `1/4 = 0.25`, not `0.250000000000`).
//!
//! This is the arithmetic substrate for the progressive EMI engine, where intermediate `BigDecimal`s (rate factors,
//! interest, balances) must be bit-for-bit identical to the original before they are quantized to currency scale.

use bigdecimal::BigDecimal;
use fc_money::decimal;

/// A Java `MathContext`: significant-figure `precision` + rounding `mode`.
#[derive(Clone, Copy, Debug)]
pub struct Mc {
    pub precision: u32,
    pub mode: bigdecimal::RoundingMode,
}

/// `BigDecimal.precision()` — the count of significant digits in the unscaled value (zero has precision 1).
fn java_precision(x: &BigDecimal) -> i64 {
    let (unscaled, _) = x.as_bigint_and_exponent();
    unscaled.magnitude().to_string().len() as i64
}

/// `floor(log10(|x|))` — position of the leading significant digit. Robust to trailing-zero padding.
fn leading_exp(x: &BigDecimal) -> i64 {
    let (_, scale) = x.as_bigint_and_exponent();
    (java_precision(x) - 1) - scale
}

fn is_zero(x: &BigDecimal) -> bool {
    decimal::compare(x, &BigDecimal::from(0)) == 0
}

impl Mc {
    #[must_use]
    pub fn new(precision: u32, mode: bigdecimal::RoundingMode) -> Self {
        Self { precision, mode }
    }

    /// `BigDecimal.round(mc)`: if `x` already has at most `precision` significant figures it is returned unchanged
    /// (no padding); otherwise it is rounded to `precision` significant figures.
    #[must_use]
    pub fn round(&self, x: &BigDecimal) -> BigDecimal {
        if java_precision(x) <= i64::from(self.precision) {
            return x.clone();
        }
        let target_scale = i64::from(self.precision) - 1 - leading_exp(x);
        decimal::set_scale(x, target_scale, self.mode)
    }

    /// `a.multiply(b, mc)` — exact product (natural scale = sum of scales), then `round(mc)`.
    #[must_use]
    pub fn mul(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        self.round(&(a * b))
    }
    /// `a.add(b, mc)`.
    #[must_use]
    pub fn add(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        self.round(&(a + b))
    }
    /// `a.subtract(b, mc)`.
    #[must_use]
    pub fn sub(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        self.round(&(a - b))
    }

    /// `a.divide(b, mc)`: the exact quotient when the division terminates within `precision` significant figures
    /// (Java's "preferred scale" — `1/4 = 0.25`), otherwise the quotient rounded to `precision` significant figures
    /// (`1/3 = 0.333333333333`). Returns zero for a zero divisor (unreachable in the engine, whose divisors —
    /// days, installment counts — are always positive).
    #[must_use]
    pub fn div(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        if is_zero(b) {
            return BigDecimal::from(0);
        }
        if is_zero(a) {
            return BigDecimal::from(0);
        }
        // Exact-termination probe: divide to a generous scale; if multiplying back reproduces `a`, it terminated.
        let probe = decimal::div(a, b, 80, self.mode);
        if decimal::compare(&(&probe * b), a) == 0 {
            let exact = probe.normalized(); // strip trailing zeros → minimal (preferred) representation
            if java_precision(&exact) <= i64::from(self.precision) {
                return exact;
            }
        }
        let target_scale = i64::from(self.precision) - 1 - leading_exp(&probe);
        decimal::div(a, b, target_scale, self.mode)
    }
}
