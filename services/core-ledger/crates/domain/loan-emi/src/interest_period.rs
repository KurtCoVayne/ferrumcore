//! Interest-period calculations — faithful port of `InterestPeriod.getCalculatedDueInterest`
//! (`.../loanproduct/calc/data/InterestPeriod.java`), the per-interest-period due-interest formula. This is one of the
//! building blocks of the (stateful) interest-schedule model; ported here as a pure function ahead of the full model.
//!
//! Currently the **declining-balance** path (`baseAmount = outstandingLoanBalance`); the flat path
//! (`baseAmount = total disbursed + capitalized income`) and the surrounding mutable model are later slices.

use bigdecimal::BigDecimal;
use fc_loan_schedule::MathCtx;
use fc_money::decimal;

fn negative_to_zero(x: BigDecimal) -> BigDecimal {
    if decimal::compare(&x, &BigDecimal::from(0)) < 0 {
        BigDecimal::from(0)
    } else {
        x
    }
}

/// `InterestPeriod.getCalculatedDueInterest()` for **declining balance** (src lines 134-158): the interest accrued on
/// the period's outstanding balance over its length, plus any credited interest, floored at zero.
///
/// `interestTillDue = outstandingBalance × rateFactorTillPeriodDueDate / lengthTillPeriodDueDate × length` (each step at
/// the `MathContext` significant figures); the result is `negativeToZero(creditedInterest + interestTillDue)`. When
/// `length_till_due` is zero the accrued portion is zero (matching the guard in `getCalculatedDueInterest(method, len)`).
#[must_use]
pub fn calculated_due_interest_declining(
    mc: &MathCtx,
    outstanding_balance: &BigDecimal,
    rate_factor_till_period_due_date: &BigDecimal,
    length: i64,
    length_till_period_due_date: i64,
    credited_interest: &BigDecimal,
) -> BigDecimal {
    let interest_till_due = if length_till_period_due_date == 0 {
        BigDecimal::from(0)
    } else {
        let per_day = mc.div(
            &mc.mul(outstanding_balance, rate_factor_till_period_due_date),
            &BigDecimal::from(length_till_period_due_date),
        );
        mc.mul(&per_day, &BigDecimal::from(length))
    };
    negative_to_zero(mc.add(credited_interest, &interest_till_due))
}
