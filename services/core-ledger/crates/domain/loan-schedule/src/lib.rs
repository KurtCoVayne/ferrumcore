//! # fc-loan-schedule
//!
//! Progressive loan schedule kernel — faithful port of the declining-balance, simple-disbursement path of the original
//! `ProgressiveEMICalculator` (`fineract-progressive-loan/.../calc/ProgressiveEMICalculator.java`), validated against
//! `ProgressiveEMICalculatorTest` (GREEN). Pure: no IO, no framework.
//!
//! Reproduces the original exactly, including (a) **Java `MathContext` significant-figure arithmetic** ([`MathCtx`])
//! for the rate-factor / EMI computation, (b) currency-scale amortization, and (c) the **equal-installment EMI
//! adjustment** ([`adjust_emi`]) that nudges the EMI by a cent when it reduces the last-vs-penultimate disparity.
//!
//! **Complete & verified for its scope:** fresh simple disbursement, declining balance, monthly periods — proven by
//! `tests/conformance_schedule.rs` against **75 golden rows / 7 scenarios** captured from the real calculator
//! (interest-free, 3–24 month terms, varied principals, `ACTUAL` and `DAYS_30`/`360`/`365` day counts).
//!
//! **Out of scope (tracked backlog, each its own golden-driven element):** multi-disbursement/tranches, interest
//! recalculation on transactions, re-age / re-amortize, interest pause, mid-term rate changes, capitalized income,
//! flat interest, non-monthly (daily/weekly) frequencies.
#![forbid(unsafe_code)]

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use bigdecimal::num_bigint::BigInt;
use fc_money::decimal;
use std::str::FromStr;

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap_or_else(|_| BigDecimal::from(0))
}
fn is_zero(x: &BigDecimal) -> bool {
    x.as_bigint_and_exponent().0 == BigInt::from(0)
}

/// Java `MathContext` (significant-figure precision + rounding mode) arithmetic.
#[derive(Clone, Copy, Debug)]
pub struct MathCtx {
    pub precision: u32,
    pub mode: RoundingMode,
}

impl MathCtx {
    /// `MathContext(precision, mode)`.
    #[must_use]
    pub fn new(precision: u32, mode: RoundingMode) -> Self {
        Self { precision, mode }
    }

    /// Round `x` to `precision` significant figures with `mode` (Java `BigDecimal.round(mc)`) — single rounding.
    #[must_use]
    pub fn round_sig(&self, x: &BigDecimal) -> BigDecimal {
        if is_zero(x) {
            return BigDecimal::from(0);
        }
        let target_scale = i64::from(self.precision) - 1 - leading_exp(x);
        decimal::set_scale(x, target_scale, self.mode)
    }

    /// `a * b` rounded to `precision` sig figs (Java `a.multiply(b, mc)`).
    #[must_use]
    pub fn mul(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        self.round_sig(&(a.clone() * b.clone()))
    }
    /// `a + b` rounded to `precision` sig figs (Java `a.add(b, mc)`).
    #[must_use]
    pub fn add(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        self.round_sig(&(a.clone() + b.clone()))
    }
    /// `a / b` correctly rounded to `precision` sig figs (Java `a.divide(b, mc)`).
    ///
    /// Single rounding of the exact quotient: a coarse division finds the quotient's magnitude, then it is divided
    /// once to the scale that yields `precision` significant figures (avoids double-rounding).
    #[must_use]
    pub fn div(&self, a: &BigDecimal, b: &BigDecimal) -> BigDecimal {
        let coarse = decimal::div(a, b, i64::from(self.precision) + 40, self.mode);
        if is_zero(&coarse) {
            return BigDecimal::from(0);
        }
        let target_scale = i64::from(self.precision) - 1 - leading_exp(&coarse);
        decimal::div(a, b, target_scale, self.mode)
    }
}

/// `floor(log10(|x|))` — position of the leading significant digit.
fn leading_exp(x: &BigDecimal) -> i64 {
    let (unscaled, scale) = x.as_bigint_and_exponent();
    let n_digits = unscaled.to_string().trim_start_matches('-').len() as i64;
    (n_digits - 1) - scale
}

/// Day-count configuration for a period's rate factor.
#[derive(Clone, Copy, Debug)]
pub struct DayCount {
    /// Days-in-year basis (e.g. 360, 365).
    pub days_in_year: i64,
    /// `true` = `DAYS_30` month basis; `false` = `ACTUAL`.
    pub days_in_month_30: bool,
}

/// One repayment period's date boundaries.
#[derive(Clone, Copy, Debug)]
pub struct PeriodBoundary {
    pub from: time::Date,
    pub due: time::Date,
}

/// Inputs for a simple-disbursement declining-balance schedule.
#[derive(Clone, Debug)]
pub struct ScheduleParams {
    /// Disbursed principal (will be normalized to `currency_scale`).
    pub disbursed: BigDecimal,
    /// Annual nominal interest rate as a percentage (e.g. `15.678`).
    pub annual_rate_pct: BigDecimal,
    pub day_count: DayCount,
    /// Currency minor-unit digits (e.g. 2).
    pub currency_scale: i64,
    pub mc: MathCtx,
}

/// One amortization row (all monetary fields at `currency_scale`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeriodResult {
    pub emi: BigDecimal,
    pub principal: BigDecimal,
    pub interest: BigDecimal,
    /// Outstanding balance AFTER this period's principal.
    pub outstanding: BigDecimal,
}

/// `rateFactorByRepaymentPeriod` (original): `rate * (multiplier*repay_every/days_in_year) * actual/calc`, then
/// fixed to 12 decimal places (`setScale(precision, mode)`).
fn rate_factor_by_repayment_period(
    mc: &MathCtx,
    rate: &BigDecimal,
    multiplier: &BigDecimal,
    repay_every: &BigDecimal,
    days_in_year: &BigDecimal,
    actual_days: &BigDecimal,
    calc_days: &BigDecimal,
) -> BigDecimal {
    if is_zero(calc_days) {
        return BigDecimal::from(0);
    }
    let fraction = mc.div(&mc.mul(multiplier, repay_every), days_in_year);
    let r = mc.div(&mc.mul(&mc.mul(rate, &fraction), actual_days), calc_days);
    decimal::set_scale(&r, 12, mc.mode)
}

/// Generate the per-period amortization for a fresh simple disbursement (declining balance).
#[must_use]
pub fn generate(params: &ScheduleParams, periods: &[PeriodBoundary]) -> Vec<PeriodResult> {
    let mc = params.mc;
    let one = BigDecimal::from(1);
    let rate = mc.div(&params.annual_rate_pct, &bd("100"));
    let diy = BigDecimal::from(params.day_count.days_in_year);

    // per-period rate factors + (1 + rateFactor)
    let mut rfs = Vec::with_capacity(periods.len());
    let mut rfp1 = Vec::with_capacity(periods.len());
    for p in periods {
        let actual_days = BigDecimal::from((p.due - p.from).whole_days());
        let rf = if params.day_count.days_in_month_30 {
            rate_factor_by_repayment_period(
                &mc,
                &rate,
                &bd("30"),
                &one,
                &diy,
                &actual_days,
                &actual_days,
            )
        } else {
            rate_factor_by_repayment_period(&mc, &rate, &actual_days, &one, &diy, &one, &one)
        };
        rfp1.push(one.clone() + rf.clone()); // 1 + rf (exact)
        rfs.push(rf);
    }

    // EMI = (rateFactorN * outstanding) / fnResult, all in MathContext sig figs, then currency-scale rounded.
    let rate_factor_n = rfp1.iter().fold(one.clone(), |acc, v| mc.mul(&acc, v));
    let fn_result = rfp1
        .iter()
        .skip(1)
        .fold(one.clone(), |f, v| mc.add(&mc.mul(&f, v), &one));
    let disbursed = decimal::set_scale(&params.disbursed, params.currency_scale, mc.mode);
    let emi_unrounded = mc.div(&mc.mul(&rate_factor_n, &disbursed), &fn_result);
    let base_emi = decimal::set_scale(&emi_unrounded, params.currency_scale, mc.mode);

    let scale = params.currency_scale;
    let mut schedule = distribute(&base_emi, &rfs, &disbursed, scale, mc.mode);
    adjust_emi(&mut schedule, &rfs, &disbursed, scale, &mc);
    schedule
}

/// Spread `base_emi` across the periods (declining balance); the last period (or any that would overpay) closes the
/// remaining balance via `emi = principal + interest`.
fn distribute(
    base_emi: &BigDecimal,
    rfs: &[BigDecimal],
    disbursed: &BigDecimal,
    scale: i64,
    mode: RoundingMode,
) -> Vec<PeriodResult> {
    let n = rfs.len();
    let mut out = Vec::with_capacity(n);
    let mut outstanding = disbursed.clone();
    for (i, rf) in rfs.iter().enumerate() {
        let interest = decimal::set_scale(&(outstanding.clone() * rf.clone()), scale, mode);
        let principal_full =
            decimal::set_scale(&(base_emi.clone() - interest.clone()), scale, mode);
        let last = i == n - 1;
        let (principal, emi_i) = if last || decimal::compare(&principal_full, &outstanding) > 0 {
            let p = outstanding.clone();
            let e = decimal::set_scale(&(p.clone() + interest.clone()), scale, mode);
            (p, e)
        } else {
            (principal_full, base_emi.clone())
        };
        outstanding = decimal::set_scale(&(outstanding.clone() - principal.clone()), scale, mode);
        out.push(PeriodResult {
            emi: emi_i,
            principal,
            interest,
            outstanding: outstanding.clone(),
        });
    }
    out
}

fn abs_bd(x: &BigDecimal) -> BigDecimal {
    if decimal::compare(x, &BigDecimal::from(0)) < 0 {
        -x.clone()
    } else {
        x.clone()
    }
}

/// Equal-installment EMI adjustment (original `checkAndAdjustEmiIfNeededOnRelatedRepaymentPeriods` + `EmiAdjustment`):
/// nudge the equal EMI by a cent (up to 3 times) when doing so reduces the last-vs-penultimate EMI disparity.
fn adjust_emi(
    schedule: &mut Vec<PeriodResult>,
    rfs: &[BigDecimal],
    disbursed: &BigDecimal,
    scale: i64,
    mc: &MathCtx,
) {
    let n = schedule.len();
    if n < 2 {
        return;
    }
    let hundred = BigDecimal::from(100);
    for _ in 0..3 {
        let penult = schedule[n - 2].emi.clone(); // originalEmi
        let emi_diff = decimal::set_scale(
            &(schedule[n - 1].emi.clone() - penult.clone()),
            scale,
            mc.mode,
        );
        if decimal::compare(&emi_diff, &BigDecimal::from(0)) == 0 {
            return;
        }
        // shouldBeAdjusted: floor(n/2) > 0 && |emiDiff|*100 > floor(n/2)
        let lower_half = (n / 2) as i64;
        if lower_half == 0 {
            return;
        }
        let lhs = abs_bd(&emi_diff) * hundred.clone();
        if decimal::compare(&lhs, &BigDecimal::from(lower_half)) <= 0 {
            return;
        }
        // adjustment = round(emiDiff / max(1, n - uncountable); uncountable=0 for a fresh schedule)
        let adjustment = decimal::set_scale(
            &mc.div(&emi_diff, &BigDecimal::from(n as i64)),
            scale,
            mc.mode,
        );
        let adjusted_emi = decimal::set_scale(&(penult.clone() + adjustment), scale, mc.mode);
        if decimal::compare(&adjusted_emi, &penult) == 0 {
            return;
        }
        let candidate = distribute(&adjusted_emi, rfs, disbursed, scale, mc.mode);
        let new_diff = abs_bd(&(candidate[n - 1].emi.clone() - candidate[n - 2].emi.clone()));
        if decimal::compare(&new_diff, &abs_bd(&emi_diff)) >= 0 {
            return; // not an improvement → keep current schedule
        }
        *schedule = candidate;
    }
}
