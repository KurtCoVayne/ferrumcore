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

#[allow(clippy::too_many_arguments)]
fn check_ip(
    model: &Model,
    i: usize,
    k: usize,
    emi: &str,
    rate_factor: &str,
    calc_interest: &str,
    cum_interest: &str,
    principal: &str,
    outstanding: &str,
) {
    assert_eq!(model.period_emi(i).to_plain_string(), emi, "rp{i} emi");
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.ip_rate_factor(i, k)),
        rate_factor,
        "rp{i} ip{k} rateFactor"
    );
    // calculatedDueInterest is full-precision; the Java goldens compare as doubles, so normalize away trailing zeros.
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.ip_calc_due_interest(i, k).normalized()),
        fc_money::decimal::to_plain_string(&bd(calc_interest).normalized()),
        "rp{i} ip{k} calculatedDueInterest"
    );
    assert_eq!(
        model.period_due_interest(i).to_plain_string(),
        cum_interest,
        "rp{i} dueInterest"
    );
    assert_eq!(
        model.period_due_principal(i).to_plain_string(),
        principal,
        "rp{i} duePrincipal"
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

/// Stage 2: the two early payments with balance corrections (the body of `test_payoff_on0215_*`). Each `payPrincipal`
/// pays more than the scheduled principal (interest is lower when paying before the due date), splitting the interest
/// period at the transaction date and cascading the recalculated balances + EMI.
#[test]
fn payoff_two_payments_with_balance_correction() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let apr1 = d(2024, Month::April, 1);
    let feb15 = d(2024, Month::February, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    // pay off period 1 (Feb1-Mar1) early on Feb15 with balance correction.
    model.pay_principal(feb1, mar1, feb15, &m("16.77"));
    model.pay_interest(feb1, mar1, feb15, &m("0.24"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.002816091954",
        "0.235340804584",
        "0.24",
        "16.77",
        "66.80",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.003017241379",
        "0.201551724117",
        "0.24",
        "16.77",
        "66.80",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.389666666644",
        "0.59",
        "16.42",
        "50.38",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.293883333317",
        "0.29",
        "16.72",
        "33.66",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.196349999989",
        "0.20",
        "16.81",
        "16.85",
    );
    check_ip(
        &model,
        5,
        0,
        "16.95",
        "0.005833333333",
        "0.098291666661",
        "0.10",
        "16.85",
        "0.00",
    );

    // totally pay off period 2 (Mar1-Apr1) on Mar1; the correction lands on period 1 (Mar1 is rp1's due date).
    model.pay_principal(mar1, apr1, mar1, &m("16.42"));
    model.pay_interest(mar1, apr1, mar1, &m("0.49"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.002816091954",
        "0.235340804584",
        "0.24",
        "16.77",
        "50.38",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.003017241379",
        "0.201551724117",
        "0.24",
        "16.77",
        "50.38",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.293883333317",
        "0.49",
        "16.52",
        "50.28",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.293299999983",
        "0.29",
        "16.72",
        "33.56",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.195766666655",
        "0.20",
        "16.81",
        "16.75",
    );
    check_ip(
        &model,
        5,
        0,
        "16.85",
        "0.005833333333",
        "0.0977083333278",
        "0.10",
        "16.75",
        "0.00",
    );
}

/// `getDueAmounts(from, due, targetDate)` on the fresh model (the `repaymentDetails*` queries of `test_payoff_on0215_*`):
/// interest accrues only up to the target date — period start → zero interest, period due → full, mid-period → prorated.
#[test]
fn payoff_get_due_amounts_fresh() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let feb15 = d(2024, Month::February, 15);

    // schedule 1st period 1st day → full principal, zero interest.
    let a = model.get_due_amounts(jan1, feb1, jan1).unwrap();
    assert_eq!(a.due_principal.to_plain_string(), "17.01");
    assert_eq!(a.due_interest.to_plain_string(), "0.00");

    // schedule 2nd period last day → full month interest.
    let b = model.get_due_amounts(feb1, mar1, mar1).unwrap();
    assert_eq!(b.due_principal.to_plain_string(), "16.52");
    assert_eq!(b.due_interest.to_plain_string(), "0.49");

    // 2nd period, mid-period payoff on Feb 15 → prorated interest, larger principal.
    let c = model.get_due_amounts(feb1, mar1, feb15).unwrap();
    assert_eq!(c.due_principal.to_plain_string(), "16.77");
    assert_eq!(c.due_interest.to_plain_string(), "0.24");
}

/// `test_fullyRepay_on0215_*`: full early payoff of every period on Feb 15 (a second, independent stateful golden that
/// exercises the early-payoff EMI override — paying future periods before their start date marks them fully paid).
#[test]
fn fully_repay_on_feb15() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let apr1 = d(2024, Month::April, 1);
    let may1 = d(2024, Month::May, 1);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let feb15 = d(2024, Month::February, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    // partial pay of period 0 with a balance correction, then check the forecast is unchanged.
    let f = model.get_due_amounts(feb1, mar1, feb15).unwrap();
    assert_eq!(f.due_principal.to_plain_string(), "16.77");
    assert_eq!(f.due_interest.to_plain_string(), "0.24");
    model.pay_principal(jan1, feb1, feb15, &m("15.0"));
    let g = model.get_due_amounts(feb1, mar1, mar1).unwrap();
    assert_eq!(g.due_principal.to_plain_string(), "16.52");
    assert_eq!(g.due_interest.to_plain_string(), "0.49");

    // full payoff of all periods on Feb 15.
    model.pay_principal(jan1, feb1, feb15, &m("1.43"));
    model.pay_interest(jan1, feb1, feb15, &m("0.58"));
    model.pay_principal(feb1, mar1, feb15, &m("16.77"));
    model.pay_interest(feb1, mar1, feb15, &m("0.24"));
    model.pay_principal(mar1, apr1, feb15, &m("17.01"));
    model.pay_principal(apr1, may1, feb15, &m("17.01"));
    model.pay_principal(may1, jun1, feb15, &m("17.01"));
    model.pay_principal(jun1, jul1, feb15, &m("15.77"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.002816091954",
        "0.235340804584",
        "0.24",
        "16.77",
        "0.00",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.003017241379",
        "0.0",
        "0.24",
        "16.77",
        "0.00",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.0",
        "0.00",
        "17.01",
        "0.00",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.0",
        "0.00",
        "17.01",
        "0.00",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.0",
        "0.00",
        "17.01",
        "0.00",
    );
    check_ip(
        &model,
        5,
        0,
        "15.77",
        "0.005833333333",
        "0.0",
        "0.00",
        "15.77",
        "0.00",
    );
    for i in 0..6 {
        assert!(model.period_fully_paid(i), "period {i} fully paid");
    }
}

/// `test_S1_full_chargeback_on_due_date_before_maturity_date`: after repaying periods 0 and 1, a full chargeback
/// (`creditPrincipal` 17.01) on Mar 1 re-adds principal — raising the outstanding via a balance correction on the
/// period ending Mar 1 and recording the credited principal on period 2 (so its due principal jumps to 33.53).
#[test]
fn chargeback_full_on_due_date() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(feb1, mar1, mar1, &m("16.52"));
    model.pay_interest(feb1, mar1, mar1, &m("0.49"));

    model.credit_principal(mar1, &m("17.01"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "84.06",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.490349999973",
        "0.49",
        "33.53",
        "50.53",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.294758333316",
        "0.29",
        "16.72",
        "33.81",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.197224999989",
        "0.20",
        "16.81",
        "17.00",
    );
    check_ip(
        &model,
        5,
        0,
        "17.10",
        "0.005833333333",
        "0.0991666666611",
        "0.10",
        "17.00",
        "0.00",
    );
}

/// `test_chargeback_principalAndInterest_*`: a chargeback of both principal (16.52) and interest (0.49) on Mar 1.
/// The credited interest is recorded on period 2's interest period, so its `getCalculatedDueInterest` becomes
/// base + credited = 0.487491666639 + 0.49 = 0.977491666638 and its cumulated due interest is 0.98.
#[test]
fn chargeback_principal_and_interest() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(feb1, mar1, mar1, &m("16.52"));
    model.pay_interest(feb1, mar1, mar1, &m("0.49"));

    model.credit_principal(mar1, &m("16.52"));
    model.credit_interest(mar1, &m("0.49"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "83.57",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.977491666638",
        "0.98",
        "33.04",
        "50.53",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.294758333316",
        "0.29",
        "16.72",
        "33.81",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.197224999989",
        "0.20",
        "16.81",
        "17.00",
    );
    check_ip(
        &model,
        5,
        0,
        "17.10",
        "0.005833333333",
        "0.0991666666611",
        "0.10",
        "17.00",
        "0.00",
    );
}

/// `test_dailyInterest_chargeback_disbursedAmt1000_*`: daily interest accrual via `getPeriodInterestTillDate` over a
/// single-period 1000 @ 7% loan (DAYS_360/30), with an interest period split at Jan 4. Each day's cumulative interest
/// (and the day's delta) matches the golden; day 31 hits maturity → the full 5.83.
#[test]
fn daily_interest_accrual() {
    let mut model = new_declining_balance(
        &[(d(2024, Month::January, 1), d(2024, Month::February, 1))],
        &bd("1000"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let jan4 = d(2024, Month::January, 4);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_interest(jan1, feb1, jan4, &m("0.56"));
    model.credit_interest(jan4, &m("0.0"));
    model.add_balance_correction(jan4, &m("0.0"));

    // (cumulative interest, day-delta) per day offset, from the Java checkDailyInterest assertions.
    let cum = [
        "0.19", "0.38", "0.56", "0.75", "0.94", "1.13", "1.32", "1.51", "1.69", "1.88", "2.07",
        "2.26", "2.45", "2.63", "2.82", "3.01", "3.20", "3.39", "3.58", "3.76", "3.95", "4.14",
        "4.33", "4.52", "4.70", "4.89", "5.08", "5.27", "5.46", "5.65", "5.83",
    ];
    let delta = [
        "0.19", "0.19", "0.18", "0.19", "0.19", "0.19", "0.19", "0.19", "0.18", "0.19", "0.19",
        "0.19", "0.19", "0.18", "0.19", "0.19", "0.19", "0.19", "0.19", "0.18", "0.19", "0.19",
        "0.19", "0.19", "0.18", "0.19", "0.19", "0.19", "0.19", "0.19", "0.18",
    ];
    for day in 1..=31u8 {
        let prev = model
            .get_period_interest_till_date(
                jan1,
                feb1,
                jan1 + time::Duration::days(i64::from(day) - 1),
                true,
            )
            .unwrap();
        let curr = model
            .get_period_interest_till_date(
                jan1,
                feb1,
                jan1 + time::Duration::days(i64::from(day)),
                true,
            )
            .unwrap();
        let idx = usize::from(day) - 1;
        assert_eq!(curr.to_plain_string(), cum[idx], "day {day} cumulative");
        assert_eq!(
            curr.minus(&prev).to_plain_string(),
            delta[idx],
            "day {day} delta"
        );
    }
}

/// `test_multi_chargeback_Amt100_*`: two `creditPrincipal` chargebacks on the same date (15.00 then 16.52) plus a
/// `creditInterest` (0.49). The credits accumulate — rp1's balance correction sums to +31.52 (outstanding 98.57) and
/// rp2's credited principal sums to 31.52 (due principal 47.96 = 49.02 emi+credited − 1.06 interest).
#[test]
fn multi_chargeback() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(feb1, mar1, mar1, &m("16.52"));
    model.pay_interest(feb1, mar1, mar1, &m("0.49"));

    model.credit_principal(mar1, &m("15.0"));
    model.credit_principal(mar1, &m("16.52"));
    model.credit_interest(mar1, &m("0.49"));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "98.57",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "1.06499166663",
        "1.06",
        "47.96",
        "50.61",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.295224999983",
        "0.30",
        "16.71",
        "33.90",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.197749999989",
        "0.20",
        "16.81",
        "17.09",
    );
    check_ip(
        &model,
        5,
        0,
        "17.19",
        "0.005833333333",
        "0.0996916666611",
        "0.10",
        "17.09",
        "0.00",
    );
}

/// `test_singleInterestPauseAmt100_*`: an interest pause Feb 5–10 splits period 1 into three interest periods
/// (Feb1-Feb4, Feb4-Feb10 paused, Feb10-Mar1). The paused interest period keeps its rate factor but contributes zero
/// calculated interest, so period 1's due interest drops to 0.39 (more principal), cascading the balances and tail EMI.
#[test]
fn single_interest_pause() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 5), d(2024, Month::February, 10));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "83.57",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.000603448276",
        "0.0504301724109",
        "0.39",
        "16.62",
        "66.95",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.001206896552",
        "0.0",
        "0.39",
        "16.62",
        "66.95",
    );
    check_ip(
        &model,
        1,
        2,
        "17.01",
        "0.004022988506",
        "0.336201149446",
        "0.39",
        "16.62",
        "66.95",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.390541666643",
        "0.39",
        "16.62",
        "50.33",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.29359166665",
        "0.29",
        "16.72",
        "33.61",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.196058333322",
        "0.20",
        "16.81",
        "16.80",
    );
    check_ip(
        &model,
        5,
        0,
        "16.90",
        "0.005833333333",
        "0.0979999999944",
        "0.10",
        "16.80",
        "0.00",
    );
}
