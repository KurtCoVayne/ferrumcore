//! Rate-factor and `fn`-value primitives — faithful port of `ProgressiveEMICalculator.rateFactorByRepaymentPeriod` /
//! `rateFactorByRepaymentEveryMonth` and `fnValue`. These are the significant-figure (`MathContext`) annuity primitives
//! the schedule generation is built on; arithmetic uses the proven [`MathCtx`] from `fc-loan-schedule`.

use bigdecimal::BigDecimal;
use fc_loan_schedule::MathCtx;
use fc_money::decimal;

fn is_zero(x: &BigDecimal) -> bool {
    decimal::compare(x, &BigDecimal::from(0)) == 0
}

/// `rateFactorByRepaymentPeriod` (src line 1950):
/// `rate × (multiplierInDays × repaymentEvery / daysInYear) × actualDays / calculatedDays`, each step rounded to the
/// `MathContext` significant figures, then fixed to `precision` decimal places (`setScale(precision, mode)`).
/// Zero when `calculatedDays` is zero.
#[must_use]
pub fn rate_factor_by_repayment_period(
    mc: &MathCtx,
    interest_rate: &BigDecimal,
    multiplier_in_days: &BigDecimal,
    repayment_every: &BigDecimal,
    days_in_year: &BigDecimal,
    actual_days_in_period: &BigDecimal,
    calculated_days_in_period: &BigDecimal,
) -> BigDecimal {
    if is_zero(calculated_days_in_period) {
        return BigDecimal::from(0);
    }
    let interest_fraction_per_period =
        mc.div(&mc.mul(multiplier_in_days, repayment_every), days_in_year);
    let r = mc.div(
        &mc.mul(
            &mc.mul(interest_rate, &interest_fraction_per_period),
            actual_days_in_period,
        ),
        calculated_days_in_period,
    );
    decimal::set_scale(&r, i64::from(mc.precision), mc.mode)
}

/// `rateFactorByRepaymentEveryMonth` (src line 1922): the month-based rate factor — `rateFactorByRepaymentPeriod` with
/// the days-in-month as the multiplier.
#[must_use]
pub fn rate_factor_by_repayment_every_month(
    mc: &MathCtx,
    interest_rate: &BigDecimal,
    repayment_every: &BigDecimal,
    days_in_month: &BigDecimal,
    days_in_year: &BigDecimal,
    actual_days_in_period: &BigDecimal,
    calculated_days_in_period: &BigDecimal,
) -> BigDecimal {
    rate_factor_by_repayment_period(
        mc,
        interest_rate,
        days_in_month,
        repayment_every,
        days_in_year,
        actual_days_in_period,
        calculated_days_in_period,
    )
}

/// `fnValue` (src line 1991): `1 + previousFnValue × currentRateFactorPlus1`, each operation at the `MathContext`
/// significant figures. The accumulating denominator of the annuity (EMI) formula.
#[must_use]
pub fn fn_value(
    mc: &MathCtx,
    previous_fn_value: &BigDecimal,
    current_rate_factor_plus_one: &BigDecimal,
) -> BigDecimal {
    mc.add(
        &BigDecimal::from(1),
        &mc.mul(previous_fn_value, current_rate_factor_plus_one),
    )
}
