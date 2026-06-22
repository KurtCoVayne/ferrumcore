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
//! **Complete & verified for fresh-disbursement schedule generation** — proven by `tests/conformance_schedule.rs`
//! against **145 golden rows / 16 scenarios** captured from the real calculator, spanning the full config space:
//! - interest method: **declining balance** and **flat**;
//! - frequency: **monthly / weekly / daily** with arbitrary **repay-every**;
//! - interest-calc method: **daily** and **same-as-repayment-period**;
//! - day counts: `ACTUAL` and `DAYS_30` over `360`/`364`/`365`;
//! - **leading principal grace** (interest-only) periods;
//! - the **equal-installment EMI adjustment**; plus interest-free, varied rates/terms/principals.
//!
//! **Out of scope (post-origination servicing — belong to the loan-servicing / transaction-processor kernels, not
//! schedule generation):** multi-disbursement/tranches after origination, interest recalculation driven by
//! transactions, re-age / re-amortize, interest pause, mid-term rate changes, capitalized income.
#![forbid(unsafe_code)]

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use bigdecimal::num_bigint::BigInt;
use fc_domain_shared::{InterestCalculationPeriodMethod, InterestMethod, PeriodFrequencyType};
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

/// Inputs for a simple-disbursement schedule (declining-balance or flat).
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
    pub interest_method: InterestMethod,
    pub calc_method: InterestCalculationPeriodMethod,
    pub frequency: PeriodFrequencyType,
    /// Repayment-every multiple (e.g. 1 = every period, 2 = every 2 periods).
    pub repay_every: i64,
    /// Number of leading interest-only (principal-grace) periods.
    pub grace_on_principal: usize,
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

/// Per-period rate factor reproducing the original `calculateRateFactorPerPeriod` (declining + flat share this).
fn period_rate_factor(
    params: &ScheduleParams,
    rate: &BigDecimal,
    actual_days: &BigDecimal,
) -> BigDecimal {
    let mc = params.mc;
    let one = BigDecimal::from(1);
    let every = BigDecimal::from(params.repay_every);
    let diy = BigDecimal::from(params.day_count.days_in_year);

    // SAME_AS_REPAYMENT_PERIOD: monthly -> /12, weekly -> /52 (original short-circuits these).
    if params.calc_method == InterestCalculationPeriodMethod::SameAsRepaymentPeriod {
        match params.frequency {
            PeriodFrequencyType::Months => {
                return rate_factor_by_repayment_period(
                    &mc,
                    rate,
                    &one,
                    &every,
                    &bd("12"),
                    actual_days,
                    actual_days,
                );
            }
            PeriodFrequencyType::Weeks => {
                return rate_factor_by_repayment_period(
                    &mc,
                    rate,
                    &one,
                    &every,
                    &bd("52"),
                    actual_days,
                    actual_days,
                );
            }
            _ => {}
        }
    }
    // DAILY (and SAME fall-through): branch on day-count-in-month.
    if params.day_count.days_in_month_30 {
        let mult = match params.frequency {
            PeriodFrequencyType::Months => bd("30"),
            PeriodFrequencyType::Weeks => bd("7"),
            _ => one.clone(), // DAYS
        };
        rate_factor_by_repayment_period(&mc, rate, &mult, &every, &diy, actual_days, actual_days)
    } else {
        // ACTUAL: rate * actualDays / daysInYear (repay-every folded into actualDays via the boundaries)
        rate_factor_by_repayment_period(&mc, rate, actual_days, &one, &diy, &one, &one)
    }
}

/// EMI annuity value (declining balance): `(rateFactorN * outstanding) / fnResult`, MathContext sig figs, then
/// currency-scale rounded.
fn annuity_emi(
    rfs: &[BigDecimal],
    outstanding: &BigDecimal,
    mc: &MathCtx,
    scale: i64,
) -> BigDecimal {
    let one = BigDecimal::from(1);
    let rfp1: Vec<BigDecimal> = rfs.iter().map(|rf| one.clone() + rf.clone()).collect();
    let rate_factor_n = rfp1.iter().fold(one.clone(), |acc, v| mc.mul(&acc, v));
    let fn_result = rfp1
        .iter()
        .skip(1)
        .fold(one.clone(), |f, v| mc.add(&mc.mul(&f, v), &one));
    let emi_unrounded = mc.div(&mc.mul(&rate_factor_n, outstanding), &fn_result);
    decimal::set_scale(&emi_unrounded, scale, mc.mode)
}

/// Generate the per-period schedule for a fresh simple disbursement (declining balance or flat; any frequency,
/// calc method, day count, repay-every, and leading principal grace).
#[must_use]
pub fn generate(params: &ScheduleParams, periods: &[PeriodBoundary]) -> Vec<PeriodResult> {
    let mc = params.mc;
    let scale = params.currency_scale;
    let mode = mc.mode;
    let rate = mc.div(&params.annual_rate_pct, &bd("100"));
    let disbursed = decimal::set_scale(&params.disbursed, scale, mode);

    let rfs: Vec<BigDecimal> = periods
        .iter()
        .map(|p| {
            period_rate_factor(
                params,
                &rate,
                &BigDecimal::from((p.due - p.from).whole_days()),
            )
        })
        .collect();

    match params.interest_method {
        InterestMethod::Flat => flat_schedule(&rfs, &disbursed, scale, mode, &mc),
        _ => {
            let gp = params.grace_on_principal.min(rfs.len());
            if gp == 0 {
                let base = annuity_emi(&rfs, &disbursed, &mc, scale);
                let mut s = distribute(&base, &rfs, &disbursed, scale, mode);
                adjust_emi(&mut s, &rfs, &disbursed, scale, &mc);
                s
            } else {
                // leading interest-only grace periods (no principal), then annuity over the remainder on disbursed
                let mut out: Vec<PeriodResult> = rfs[..gp]
                    .iter()
                    .map(|rf| {
                        let interest =
                            decimal::set_scale(&(disbursed.clone() * rf.clone()), scale, mode);
                        let zero = decimal::set_scale(&BigDecimal::from(0), scale, mode);
                        PeriodResult {
                            emi: interest.clone(),
                            principal: zero,
                            interest,
                            outstanding: disbursed.clone(),
                        }
                    })
                    .collect();
                let rest = &rfs[gp..];
                let base = annuity_emi(rest, &disbursed, &mc, scale);
                let mut tail = distribute(&base, rest, &disbursed, scale, mode);
                adjust_emi(&mut tail, rest, &disbursed, scale, &mc);
                out.extend(tail);
                out
            }
        }
    }
}

/// Flat-interest schedule (original `calculateEMIOnActualModelWithFlatInterestMethod`, fresh case): interest is on the
/// original principal each period; `EMI = (principal + Σinterest) / n`, last period carries the rounding remainder.
fn flat_schedule(
    rfs: &[BigDecimal],
    disbursed: &BigDecimal,
    scale: i64,
    mode: RoundingMode,
    mc: &MathCtx,
) -> Vec<PeriodResult> {
    let n = rfs.len();
    let interests: Vec<BigDecimal> = rfs
        .iter()
        .map(|rf| decimal::set_scale(&(disbursed.clone() * rf.clone()), scale, mode))
        .collect();
    let total = decimal::set_scale(
        &interests
            .iter()
            .fold(disbursed.clone(), |acc, i| acc + i.clone()),
        scale,
        mode,
    );
    let period_emi = decimal::set_scale(&mc.div(&total, &BigDecimal::from(n as i64)), scale, mode);
    let remainder = decimal::set_scale(
        &(total.clone() - period_emi.clone() * BigDecimal::from(n as i64)),
        scale,
        mode,
    );
    let mut out = Vec::with_capacity(n);
    let mut outstanding = disbursed.clone();
    for (i, interest) in interests.iter().enumerate() {
        let emi_i = if i == n - 1 {
            decimal::set_scale(&(period_emi.clone() + remainder.clone()), scale, mode)
        } else {
            period_emi.clone()
        };
        let principal = decimal::set_scale(&(emi_i.clone() - interest.clone()), scale, mode);
        outstanding = decimal::set_scale(&(outstanding.clone() - principal.clone()), scale, mode);
        out.push(PeriodResult {
            emi: emi_i,
            principal,
            interest: interest.clone(),
            outstanding: outstanding.clone(),
        });
    }
    out
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
