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

/// `test_balance_correction_on0215_*` (tail): after the two early payments, `getDueAmounts` on the last period at its
/// due date and after it both return 16.75 / 0.10; then a zero balance correction on Jun 10 splits the last period's
/// interest period into Jun1-Jun10 and Jun10-Jul1.
#[test]
fn last_period_due_amounts_and_zero_correction_split() {
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
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let feb15 = d(2024, Month::February, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(feb1, mar1, feb15, &m("16.77"));
    model.pay_interest(feb1, mar1, feb15, &m("0.24"));
    model.pay_principal(mar1, apr1, mar1, &m("16.42"));
    model.pay_interest(mar1, apr1, mar1, &m("0.49"));

    // last period due amounts on its due date and after maturity.
    let on_due = model.get_due_amounts(jun1, jul1, jul1).unwrap();
    assert_eq!(on_due.due_principal.to_plain_string(), "16.75");
    assert_eq!(on_due.due_interest.to_plain_string(), "0.10");
    let after = model
        .get_due_amounts(jun1, jul1, d(2024, Month::July, 15))
        .unwrap();
    assert_eq!(after.due_principal.to_plain_string(), "16.75");
    assert_eq!(after.due_interest.to_plain_string(), "0.10");

    // zero balance correction on Jun 10 splits the last period's interest period.
    model.add_balance_correction(d(2024, Month::June, 10), &m("0.0"));
    let last = &model.periods[5].interest_periods;
    assert_eq!(last.len(), 2, "last period split into 2 interest periods");
    assert_eq!(last[0].from, jun1);
    assert_eq!(last[0].due, d(2024, Month::June, 10));
    assert_eq!(last[1].from, d(2024, Month::June, 10));
    assert_eq!(last[1].due, jul1);
}

/// `test_multidisbursement_total_repay1st_*`: disburse 300, fully repay all six periods on the disbursement date, then
/// a second disbursement of 200 on the same date. The second disbursement recomputes the EMI via the new-model merge
/// (paid-amount-free copy) — total due interest 4.11, EMI on the still-open periods 85.04 then 78.91 on the last.
#[test]
fn multi_disbursement_after_full_repay() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("300"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jan1 = d(2024, Month::January, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    // fully repay all six periods on Jan 1 (51.03 x5 + 44.85 = 300).
    for (i, (from, due)) in six_monthly().iter().enumerate() {
        let amt = if i == 5 { "44.85" } else { "51.03" };
        model.pay_principal(*from, *due, jan1, &m(amt));
    }
    assert_eq!(
        model.total_due_interest_amount().to_plain_string(),
        "0.00",
        "all interest paid off"
    );

    // second disbursement of 200 on Jan 1.
    model.add_disbursement(jan1, &m("200.0"));
    assert_eq!(
        model.total_due_interest_amount().to_plain_string(),
        "4.11",
        "total due interest after 2nd disbursement"
    );
    assert_eq!(
        model.period_emi(4).to_plain_string(),
        "85.04",
        "period 4 EMI"
    );
    assert_eq!(
        model.period_emi(5).to_plain_string(),
        "78.91",
        "period 5 EMI"
    );
}

/// `test_actual_actual_repayment_schedule_across_multiple_years_overdue`: 5000 @ 9.99% ACTUAL/ACTUAL, six monthly
/// periods crossing 2023→2024. Each period's due principal is added back as a positive balance correction (overdue:
/// nothing is actually paid down), so the outstanding stays 5000 throughout and the per-period interest varies with the
/// month length; the last EMI grows to 960.01. Exercises getDueAmounts + addBalanceCorrection with positive amounts.
#[test]
fn overdue_principal_added_back_each_period() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let mut model = new_declining_balance_with_day_count(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCount::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    for &(from, due) in &periods {
        let due_amounts = model.get_due_amounts(from, due, due).unwrap();
        let dp = due_amounts.due_principal.clone();
        model.add_balance_correction(due, &dp);
    }

    check_rp(&model, 0, "857.71", "41.05", "816.66", "5000.00");
    check_rp(&model, 1, "857.71", "42.37", "815.34", "5000.00");
    check_rp(&model, 2, "857.71", "42.31", "815.40", "5000.00");
    check_rp(&model, 3, "857.71", "39.58", "818.13", "5000.00");
    check_rp(&model, 4, "857.71", "42.31", "815.40", "5000.00");
    check_rp(&model, 5, "960.01", "40.94", "919.07", "5000.00");
}

/// `test_actual_actual_repayment_schedule_across_multiple_years` (payment block): 5000 @ 9.99% ACTUAL/ACTUAL, six
/// monthly periods crossing 2023→2024; period 0 paid early on Dec 10 (820.76 principal + 36.95 interest = full EMI).
/// Period 0 becomes fully paid, and the reduced early interest cascades to the remaining periods. Stateful ACTUAL/ACTUAL.
#[test]
fn actual_actual_early_payment() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let mut model = new_declining_balance_with_day_count(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCount::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    let nov13 = d(2023, Month::November, 13);
    let dec13 = d(2023, Month::December, 13);
    let dec10 = d(2023, Month::December, 10);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfUp, &bd(s));
    model.pay_principal(nov13, dec13, dec10, &m("820.76"));
    model.pay_interest(nov13, dec13, dec10, &m("36.95"));

    check_rp(&model, 0, "857.71", "36.95", "820.76", "4179.24");
    assert!(model.period_fully_paid(0), "period 0 fully paid");
    check_rp(&model, 1, "857.71", "38.85", "818.86", "3360.38");
    check_rp(&model, 2, "857.71", "28.43", "829.28", "2531.10");
    check_rp(&model, 3, "857.71", "20.04", "837.67", "1693.43");
    check_rp(&model, 4, "857.71", "14.33", "843.38", "850.05");
    check_rp(&model, 5, "857.01", "6.96", "850.05", "0.00");
}

/// `test_S4_full_chargeback_in_middle_of_instalment_before_maturity_date`: after repaying periods 0-1, a full chargeback
/// (17.01) MID-period on Mar 15 — the balance correction lands on period 2's Mar1-Mar15 interest period and the
/// credited principal on its Mar15-Apr1 interest period (so period 2 splits into two), due principal 33.57.
#[test]
fn chargeback_mid_instalment() {
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
    let mar15 = d(2024, Month::March, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(feb1, mar1, mar1, &m("16.52"));
    model.pay_interest(feb1, mar1, mar1, &m("0.49"));

    model.credit_principal(mar15, &m("17.01"));

    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "67.05",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.002634408602",
        "0.176637096765",
        "0.45",
        "33.57",
        "50.49",
    );
    check_ip(
        &model,
        2,
        1,
        "17.01",
        "0.003198924731",
        "0.268901612888",
        "0.45",
        "33.57",
        "50.49",
    );
    check_rp(&model, 3, "17.01", "0.29", "16.72", "33.77");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "16.96");
    check_rp(&model, 5, "17.06", "0.10", "16.96", "0.00");
}

/// `test_s5_chargeback_in_period_*`: a mid-period chargeback (16.52 principal + 0.49 interest) on Mar 15 after repaying
/// periods 0-1, then `getDueAmounts` at the mid-period target Mar 15. The live period-2 due principal is 33.09, while
/// the recalc-till-Mar-15 due amounts are 33.35 / 0.67 (interest accrued only to Mar 15).
#[test]
fn chargeback_mid_period_then_get_due_amounts() {
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
    let mar15 = d(2024, Month::March, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(feb1, mar1, mar1, &m("16.52"));
    model.pay_interest(feb1, mar1, mar1, &m("0.49"));

    model.credit_principal(mar15, &m("16.52"));
    model.credit_interest(mar15, &m("0.49"));

    // live model: period 2 due principal 33.09.
    assert_eq!(model.period_due_principal(2).to_plain_string(), "33.09");
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.002634408602",
        "0.176637096765",
        "0.93",
        "33.09",
        "50.48",
    );
    check_ip(
        &model,
        2,
        1,
        "17.01",
        "0.003198924731",
        "0.75733413977",
        "0.93",
        "33.09",
        "50.48",
    );
    check_rp(&model, 3, "17.01", "0.29", "16.72", "33.76");
    check_rp(&model, 5, "17.05", "0.10", "16.95", "0.00");

    // getDueAmounts at the mid-period target Mar 15 (interest accrued only to Mar 15).
    let due = model.get_due_amounts(mar1, apr1, mar15).unwrap();
    assert_eq!(due.due_principal.to_plain_string(), "33.35");
    assert_eq!(due.due_interest.to_plain_string(), "0.67");
}

/// `test_repayment_actual_actual_repayment_schedule_across_multiple_years_overdue`: 5000 @ 9.99% ACTUAL/ACTUAL; period 0
/// paid early (36.95 interest + 820.76 principal on Dec 10), then periods 1-5 go overdue (due principal added back each
/// period). Outstanding holds at 4179.24 after period 0; last EMI grows to 925.27. Combines early payment + overdue.
#[test]
fn actual_actual_early_payment_then_overdue() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let mut model = new_declining_balance_with_day_count(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCount::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfUp, &bd(s));
    model.pay_interest(
        periods[0].0,
        periods[0].1,
        d(2023, Month::December, 10),
        &m("36.95"),
    );
    model.pay_principal(
        periods[0].0,
        periods[0].1,
        d(2023, Month::December, 10),
        &m("820.76"),
    );
    for &(from, due) in &periods[1..] {
        let dp = model.get_due_amounts(from, due, due).unwrap().due_principal;
        model.add_balance_correction(due, &dp);
    }

    check_rp(&model, 0, "857.71", "36.95", "820.76", "4179.24");
    assert!(model.period_fully_paid(0), "period 0 fully paid");
    check_rp(&model, 1, "857.71", "38.85", "818.86", "4179.24");
    check_rp(&model, 2, "857.71", "35.36", "822.35", "4179.24");
    check_rp(&model, 3, "857.71", "33.08", "824.63", "4179.24");
    check_rp(&model, 4, "857.71", "35.36", "822.35", "4179.24");
    check_rp(&model, 5, "925.27", "34.22", "891.05", "4179.24");
}

/// `getOutstandingAmountsTillDate` (used by the re-age / EQUAL_AMORTIZATION flow): total outstanding principal and
/// interest accrued up to a date. On the fresh 100 @ 7% schedule, till maturity = (100.00, 2.05); after fully repaying
/// period 0 it drops to (83.57, 1.47).
#[test]
fn outstanding_amounts_till_date() {
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
    let jul1 = d(2024, Month::July, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let (p0, i0) = model.get_outstanding_amounts_till_date(jul1);
    assert_eq!(
        p0.to_plain_string(),
        "100.00",
        "fresh outstanding principal"
    );
    assert_eq!(i0.to_plain_string(), "2.05", "fresh outstanding interest");

    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));

    let (p1, i1) = model.get_outstanding_amounts_till_date(jul1);
    assert_eq!(
        p1.to_plain_string(),
        "83.57",
        "outstanding principal after period 0"
    );
    assert_eq!(
        i1.to_plain_string(),
        "1.47",
        "outstanding interest after period 0"
    );
}

/// `test_transactionInMiddleOfPeriod_EQUAL_AMORTIZATION_FULL_INTEREST_noTransactionTilDate_noInterestRecalc`: the exact
/// golden — 100 @ 15.678% DAYS_365/ACTUAL generates EMI 17.43 (interests 1.33/1.05/0.90/0.66/0.46/0.22), and re-aging
/// on Apr 15 into 6 equal-amortization installments from Apr 20 (no payments) leaves the total outstanding
/// (100.00 / 4.62) unchanged. Now uses the real DAYS_365 day count via recalc's ActualFixedYear.
#[test]
fn reage_equal_amortization_preserves_outstanding() {
    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );

    // generation block (matches the Java golden exactly).
    check_rp(&model, 0, "17.43", "1.33", "16.10", "83.90");
    check_rp(&model, 1, "17.43", "1.05", "16.38", "67.52");
    check_rp(&model, 2, "17.43", "0.90", "16.53", "50.99");
    check_rp(&model, 3, "17.43", "0.66", "16.77", "34.22");
    check_rp(&model, 4, "17.43", "0.46", "16.97", "17.25");
    check_rp(&model, 5, "17.47", "0.22", "17.25", "0.00");

    let maturity_before = model.periods.last().unwrap().due;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(maturity_before);
    assert_eq!(
        p_before.to_plain_string(),
        "100.00",
        "outstanding principal before re-age"
    );
    assert_eq!(
        i_before.to_plain_string(),
        "4.62",
        "outstanding interest before re-age"
    );

    model.re_age_equal_amortization(
        d(2024, Month::April, 15),
        d(2024, Month::April, 20),
        6,
        1,
        true,
    );

    let maturity_after = model.periods.last().unwrap().due;
    let (p_after, i_after) = model.get_outstanding_amounts_till_date(maturity_after);

    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "outstanding principal unchanged by re-age"
    );
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "outstanding interest unchanged by re-age"
    );
}

/// Re-age invariant **after payments** (the assertion in `test_transactionInMiddleOfPeriod_EQUAL_AMORTIZATION_FULL_INTEREST`,
/// which the recalc day-count enum can't model exactly, but the invariant is day-count/payment-agnostic): re-aging a
/// partially-paid loan still leaves the *remaining* total outstanding unchanged. Periods 0 and 1 are repaid, then the
/// loan is re-aged on Apr 15 into 6 installments — stresses calculate_paid_balances_after_date +
/// create_repayment_period_for_early_repaid with non-zero paid amounts.
#[test]
fn reage_after_payments_preserves_outstanding() {
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

    let maturity_before = model.periods.last().unwrap().due;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(maturity_before);
    // sanity: 2 of 6 periods repaid, so outstanding principal < 100.
    assert_eq!(
        p_before.to_plain_string(),
        "67.05",
        "outstanding principal after 2 payments"
    );

    model.re_age_equal_amortization(
        d(2024, Month::April, 15),
        d(2024, Month::April, 20),
        6,
        1,
        true,
    );

    let maturity_after = model.periods.last().unwrap().due;
    let (p_after, i_after) = model.get_outstanding_amounts_till_date(maturity_after);
    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "outstanding principal unchanged by re-age (after payments)"
    );
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "outstanding interest unchanged by re-age (after payments)"
    );
}

/// `test_transactionInMiddleOfPeriod_EQUAL_EQUAL_AMORTIZATION_PAYABLE_INTEREST`: 100 @ 15.678% DAYS_365, a complex
/// payment sequence (on-time period 0, MIR overpayments of periods 1/4/5 on Feb 1, two balance corrections via
/// getDueAmounts), then a PAYABLE_INTEREST re-age on Apr 15. Invariant: outstanding-till-transaction-date before the
/// re-age equals outstanding-till-maturity after it. Exercises the full op set + re-age with full_interest=false.
#[test]
fn reage_payable_interest_after_mir_payments() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let apr1 = d(2024, Month::April, 1);
    let may1 = d(2024, Month::May, 1);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let apr15 = d(2024, Month::April, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );

    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));
    model.pay_principal(feb1, mar1, feb1, &m("17.43"));
    model.pay_principal(jun1, jul1, feb1, &m("17.43"));
    model.pay_principal(may1, jun1, feb1, &m("2.57"));

    let dp1 = model
        .get_due_amounts(mar1, apr1, apr1)
        .unwrap()
        .due_principal;
    model.add_balance_correction(apr1, &dp1);
    let dp2 = model
        .get_due_amounts(mar1, apr1, apr1)
        .unwrap()
        .due_principal;
    model.add_balance_correction(apr15, &m("0").minus(&dp2));

    let (p_before, i_before) = model.get_outstanding_amounts_till_date(apr15);
    model.re_age_equal_amortization(apr15, d(2024, Month::April, 20), 6, 1, false);
    let maturity_after = model.periods.last().unwrap().due;
    let (p_after, i_after) = model.get_outstanding_amounts_till_date(maturity_after);

    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "PAYABLE_INTEREST re-age preserves outstanding principal"
    );
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "PAYABLE_INTEREST re-age preserves outstanding interest"
    );
}

/// `test_chargeBackOn2ndRP_ReAgeingOn1stRPsDueDate_EQUAL_AMORTIZATION_FULL_INTEREST`: 100 @ 15.678% DAYS_365 — repay
/// period 0, chargeback 17.43 on Feb 1, then a FULL_INTEREST re-age on Feb 1. The chargeback + re-age together still
/// preserve the total outstanding (before == after). Exercises re-age interacting with credited principal.
#[test]
fn reage_after_chargeback_preserves_outstanding() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );

    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));
    model.credit_principal(feb1, &m("17.43"));

    let maturity_before = model.periods.last().unwrap().due;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(maturity_before);

    model.re_age_equal_amortization(feb1, feb1, 6, 1, true);

    let maturity_after = model.periods.last().unwrap().due;
    let (p_after, i_after) = model.get_outstanding_amounts_till_date(maturity_after);
    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "re-age after chargeback preserves outstanding principal"
    );
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "re-age after chargeback preserves outstanding interest"
    );
}
