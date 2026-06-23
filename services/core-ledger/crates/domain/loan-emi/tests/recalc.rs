//! Conformance for the stateful recalculation engine ([`fc_loan_emi::recalc`]), replaying the VERIFIED
//! `ProgressiveEMICalculatorTest.test_payoff_on0215_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month`
//! family: 100 disbursed over six monthly periods @ 7% nominal, DAYS_360 / DAYS_30, HALF_EVEN. Stage 1 verifies the
//! fresh disbursement through the mutable model (before any payment); later stages add the payments.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::recalc::{
    DayCount, Model, new_declining_balance, new_declining_balance_with_day_count,
};
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
    // rateFactor and calculatedDueInterest compare as doubles in Java, so normalize away trailing zeros (e.g. the
    // disbursement-stub / paused boundary factors render as 0.000000000000 here but 0.0 in the golden).
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.ip_rate_factor(i, k).normalized()),
        fc_money::decimal::to_plain_string(&bd(rate_factor).normalized()),
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

/// `test_interestPauseBetweenTwoPeriodsAmt100_*`: a pause Feb 10 – Mar 10 spans periods 1 and 2, splitting each into a
/// real + paused interest period. Confirms `apply_interest_pause` handles a multi-period pause (clamping the window to
/// each period and skipping the boundary insert when an interest period already starts/ends there).
#[test]
fn interest_pause_between_two_periods() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 10), d(2024, Month::March, 10));

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
        "0.001609195402",
        "0.134480459762",
        "0.13",
        "16.88",
        "66.69",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.004224137931",
        "0.0",
        "0.13",
        "16.88",
        "66.69",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.001693548387",
        "0.0",
        "0.28",
        "16.73",
        "49.96",
    );
    check_ip(
        &model,
        2,
        1,
        "17.01",
        "0.004139784946",
        "0.276082258049",
        "0.28",
        "16.73",
        "49.96",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.291433333317",
        "0.29",
        "16.72",
        "33.24",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.193899999989",
        "0.19",
        "16.82",
        "16.42",
    );
    check_ip(
        &model,
        5,
        0,
        "16.52",
        "0.005833333333",
        "0.095783333328",
        "0.10",
        "16.42",
        "0.00",
    );
}

/// `test_interestPauseLastDayOfMonthAmt100_*`: a pause Jan 10 – 31 inside period 0 — confirms the pause splits coexist
/// with the disbursement stub (rp0 ends up with stub + Jan1-Jan9 + Jan9-Jan31(paused) + Jan31-Feb1).
#[test]
fn interest_pause_inside_first_period_with_stub() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::January, 10), d(2024, Month::January, 31));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.17", "16.84", "83.16",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.001505376344",
        "0.1505376344",
        "0.17",
        "16.84",
        "83.16",
    );
    check_ip(
        &model,
        0,
        2,
        "17.01",
        "0.004139784946",
        "0.0",
        "0.17",
        "16.84",
        "83.16",
    );
    check_ip(
        &model,
        0,
        3,
        "17.01",
        "0.000188172043",
        "0.0188172043",
        "0.17",
        "16.84",
        "83.16",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.485099999971",
        "0.49",
        "16.52",
        "66.64",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.38873333331",
        "0.39",
        "16.62",
        "50.02",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.291783333317",
        "0.29",
        "16.72",
        "33.30",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.194249999989",
        "0.19",
        "16.82",
        "16.48",
    );
    check_ip(
        &model,
        5,
        0,
        "16.58",
        "0.005833333333",
        "0.0961333333278",
        "0.10",
        "16.48",
        "0.00",
    );
}

/// `test_interestPauseWholeMonthAmt100_*`: a pause Feb 1 – 29 (whole month, on the rp0/rp1 boundary). The pause start
/// (Feb 1 − 1 = Jan 31) splits period 0's tail and the pause runs into period 1, leaving period 1 with almost no
/// accrual (due interest 0.02).
#[test]
fn interest_pause_whole_month() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 1), d(2024, Month::February, 29));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.56", "16.45", "83.55",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.00564516129",
        "0.564516129",
        "0.56",
        "16.45",
        "83.55",
    );
    check_ip(
        &model,
        0,
        2,
        "17.01",
        "0.000188172043",
        "0.0",
        "0.56",
        "16.45",
        "83.55",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005632183908",
        "0.0",
        "0.02",
        "16.99",
        "66.56",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.000201149425",
        "0.0168060344588",
        "0.02",
        "16.99",
        "66.56",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.388266666645",
        "0.39",
        "16.62",
        "49.94",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.29131666665",
        "0.29",
        "16.72",
        "33.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.193783333322",
        "0.19",
        "16.82",
        "16.40",
    );
    check_ip(
        &model,
        5,
        0,
        "16.50",
        "0.005833333333",
        "0.0956666666613",
        "0.10",
        "16.40",
        "0.00",
    );
}

/// `test_disbursedAmt1000_actual_actual_repayEvery1Month_verify_due_principal_amounts`: 1000 @ 25% ACTUAL/ACTUAL over
/// 12 monthly periods (crossing into 2025), paying each period's principal upfront on the disbursement date. Confirms
/// the ACTUAL/ACTUAL day count is wired into the recalc rate factors (EMI 95.02), with interest zero (paid before
/// accrual) and every period fully paid.
#[test]
fn actual_actual_pay_all_upfront() {
    let periods: Vec<(Date, Date)> = {
        let mut v = Vec::new();
        let months = [
            (2024, Month::January),
            (2024, Month::February),
            (2024, Month::March),
            (2024, Month::April),
            (2024, Month::May),
            (2024, Month::June),
            (2024, Month::July),
            (2024, Month::August),
            (2024, Month::September),
            (2024, Month::October),
            (2024, Month::November),
            (2024, Month::December),
            (2025, Month::January),
        ];
        for w in months.windows(2) {
            v.push((d(w[0].0, w[0].1, 1), d(w[1].0, w[1].1, 1)));
        }
        v
    };
    let mut model = new_declining_balance_with_day_count(
        &periods,
        &bd("1000"),
        &bd("25"),
        DayCount::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    let disb = d(2024, Month::January, 1);
    for &(from, due) in &periods {
        let due_amounts = model.get_due_amounts(from, due, disb).unwrap();
        if due_amounts.due_principal.is_greater_than_zero() {
            let dp = due_amounts.due_principal.clone();
            model.pay_principal(from, due, disb, &dp);
        }
    }

    for i in 0..10 {
        check_rp(&model, i, "95.02", "0.00", "95.02", "0.00");
        assert!(model.period_fully_paid(i), "period {i} fully paid");
    }
    check_rp(&model, 10, "49.80", "0.00", "49.80", "0.00");
    check_rp(&model, 11, "0.00", "0.00", "0.00", "0.00");
}

/// `test_S2_S3_partial_and_full_chargeback_on_due_date_before_maturity_date`: after repaying periods 0-1, a partial
/// chargeback (15.00) then a full chargeback (17.01) on Mar 1, asserting after each. The credited principals accumulate
/// on period 2 (due principal 31.53 then 48.44) and the balance corrections accumulate on period 1.
#[test]
fn chargeback_partial_then_full() {
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

    // partial chargeback 15.00
    model.credit_principal(mar1, &m("15.0"));
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "82.05",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.478624999974",
        "0.48",
        "31.53",
        "50.52",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.294699999983",
        "0.29",
        "16.72",
        "33.80",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.197166666655",
        "0.20",
        "16.81",
        "16.99",
    );
    check_ip(
        &model,
        5,
        0,
        "17.09",
        "0.005833333333",
        "0.0991083333276",
        "0.10",
        "16.99",
        "0.00",
    );

    // full chargeback 17.01
    model.credit_principal(mar1, &m("17.01"));
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "99.06",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.577849999968",
        "0.58",
        "48.44",
        "50.62",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.295283333316",
        "0.30",
        "16.71",
        "33.91",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.197808333322",
        "0.20",
        "16.81",
        "17.10",
    );
    check_ip(
        &model,
        5,
        0,
        "17.20",
        "0.005833333333",
        "0.0997499999943",
        "0.10",
        "17.10",
        "0.00",
    );
}

/// `test_reschedule_interest_on0120_*`: pay the last period early (17.01 on Jan 15), then change the interest rate to
/// 4% effective Jan 20 (− 1 day = Jan 19). Period 0 ends up with 4 interest periods (stub, Jan1-Jan15 @7%,
/// Jan15-Jan19 @7%, Jan19-Feb1 @4%); the annuity EMI + cent-adjust loop settle at 16.80 and the pre-paid last period
/// stays at 17.01. Exercises change_interest_rate (variable rate) + the getEmiAdjustment cent-adjust loop.
#[test]
fn change_interest_rate_with_prepaid_last_period() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let jan15 = d(2024, Month::January, 15);
    let jan20 = d(2024, Month::January, 20);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jun1, jul1, jan15, &m("17.01"));
    model.change_interest_rate(jan20, &bd("4.0"));

    check_ip(
        &model, 0, 0, "16.80", "0.0", "0.0", "0.44", "16.36", "66.63",
    );
    check_ip(
        &model,
        0,
        1,
        "16.80",
        "0.002634408602",
        "0.2634408602",
        "0.44",
        "16.36",
        "66.63",
    );
    check_ip(
        &model,
        0,
        2,
        "16.80",
        "0.000752688172",
        "0.0624655913944",
        "0.44",
        "16.36",
        "66.63",
    );
    check_ip(
        &model,
        0,
        3,
        "16.80",
        "0.001397849462",
        "0.116007526851",
        "0.44",
        "16.36",
        "66.63",
    );
    check_rp(&model, 1, "16.80", "0.22", "16.58", "50.05");
    check_rp(&model, 2, "16.80", "0.17", "16.63", "33.42");
    check_rp(&model, 3, "16.80", "0.11", "16.69", "16.73");
    check_rp(&model, 4, "16.79", "0.06", "16.73", "0.00");
    check_rp(&model, 5, "17.01", "0.00", "17.01", "0.00");
}

/// `test_reschedule_interest_on0215_4per_*`: a clean interest-rate change to 4% effective Feb 15 (− 1 day = Feb 14),
/// no prior payments. Period 1 splits into Feb1-Feb14 @7% and Feb14-Mar1 @4%; the blended annuity EMI is a flat 16.90.
#[test]
fn change_interest_rate_clean() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.change_interest_rate(d(2024, Month::February, 15), &bd("4.0"));

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
        "16.90",
        "0.002614942529",
        "0.218530747114",
        "0.37",
        "16.53",
        "67.04",
    );
    check_ip(
        &model,
        1,
        1,
        "16.90",
        "0.001839080460",
        "0.153691954042",
        "0.37",
        "16.53",
        "67.04",
    );
    check_rp(&model, 2, "16.90", "0.22", "16.68", "50.36");
    check_rp(&model, 3, "16.90", "0.17", "16.73", "33.63");
    check_rp(&model, 4, "16.90", "0.11", "16.79", "16.84");
    check_rp(&model, 5, "16.90", "0.06", "16.84", "0.00");
}
