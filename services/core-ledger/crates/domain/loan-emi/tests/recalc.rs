//! Conformance for the stateful recalculation engine ([`fc_loan_emi::recalc`]), replaying the VERIFIED
//! `ProgressiveEMICalculatorTest.test_payoff_on0215_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month`
//! family: 100 disbursed over six monthly periods @ 7% nominal, DAYS_360 / DAYS_30, HALF_EVEN. Stage 1 verifies the
//! fresh disbursement through the mutable model (before any payment); later stages add the payments.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::recalc::{Model, new_declining_balance};
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn d(year: i32, month: Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).unwrap()
}

fn check_rp(
    model: &Model,
    i: usize,
    emi: &str,
    interest: &str,
    principal: &str,
    outstanding: &str,
) {
    assert_eq!(model.period_emi(i).to_plain_string(), emi, "rp{i} emi");
    assert_eq!(
        model.period_due_interest(i).to_plain_string(),
        interest,
        "rp{i} interest"
    );
    assert_eq!(
        model.period_due_principal(i).to_plain_string(),
        principal,
        "rp{i} principal"
    );
    assert_eq!(
        model.period_outstanding(i).to_plain_string(),
        outstanding,
        "rp{i} outstanding"
    );
}

fn six_monthly() -> Vec<(Date, Date)> {
    vec![
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ]
}

/// Stage 1: fresh disbursement reproduces the declining-balance schedule (EMI 17.01, interest 0.58 on period 0, etc.).
#[test]
fn payoff_fresh_disbursement() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    // rp0 has two interest periods: the disbursement stub (Jan1-Jan1) then the real one (Jan1-Feb1).
    assert_eq!(
        model.periods[0].interest_periods.len(),
        2,
        "rp0 has stub + real interest period"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.ip_rate_factor(0, 1)),
        "0.005833333333"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.ip_calc_due_interest(0, 1)),
        "0.5833333333"
    );

    check_rp(&model, 0, "17.01", "0.58", "16.43", "83.57");
    check_rp(&model, 1, "17.01", "0.49", "16.52", "67.05");
    check_rp(&model, 2, "17.01", "0.39", "16.62", "50.43");
    check_rp(&model, 3, "17.01", "0.29", "16.72", "33.71");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "16.90");
    check_rp(&model, 5, "17.00", "0.10", "16.90", "0.00");
}
