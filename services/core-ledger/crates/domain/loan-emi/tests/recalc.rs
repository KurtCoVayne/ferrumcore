//! Conformance for the stateful recalculation engine ([`fc_loan_emi::recalc`]), replaying the VERIFIED
//! `ProgressiveEMICalculatorTest.test_payoff_on0215_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month`
//! family: 100 disbursed over six monthly periods @ 7% nominal, DAYS_360 / DAYS_30, HALF_EVEN. Stage 1 verifies the
//! fresh disbursement through the mutable model (before any payment); later stages add the payments.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::recalc::{
    DayCount, InterestMethod, Model, new_declining_balance, new_declining_balance_with_day_count,
    new_empty, new_empty_same_as_repayment,
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

/// `test_interestPauseFirstDayOfMonthAmt100_*`: pause Feb 1–5 — the pause starts on period 1's from-date (a period
/// boundary), so period 0 is split at Jan 31 (pauseStart−1) leaving a 1-day non-accruing stub, and period 1 splits at
/// Feb 4 with the Feb1-Feb4 segment paused.
#[test]
fn interest_pause_first_day_of_month() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 1), d(2024, Month::February, 5));

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
        "0.000804597701",
        "0.0",
        "0.42",
        "16.59",
        "66.96",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.005028735632",
        "0.420150862055",
        "0.42",
        "16.59",
        "66.96",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.390599999978",
        "0.39",
        "16.62",
        "50.34",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.293649999983",
        "0.29",
        "16.72",
        "33.62",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.196116666655",
        "0.20",
        "16.81",
        "16.81",
    );
    check_ip(
        &model,
        5,
        0,
        "16.91",
        "0.005833333333",
        "0.0980583333276",
        "0.10",
        "16.81",
        "0.00",
    );
}

/// `test_interestPauseTwoWholeMonthsAmt100_*`: pause Feb 1 – Mar 31 spanning periods 1 and 2 entirely — period 1
/// accrues no interest (full pause), period 2 carries only the Mar 31–Apr 1 tail, cascading balances and tail EMI.
#[test]
fn interest_pause_two_whole_months() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 1), d(2024, Month::March, 31));

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
        "0.005833333333",
        "0.0",
        "0.00",
        "17.01",
        "66.54",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.00564516129",
        "0.0",
        "0.01",
        "17.00",
        "49.54",
    );
    check_ip(
        &model,
        2,
        1,
        "17.01",
        "0.000188172043",
        "0.0125209677412",
        "0.01",
        "17.00",
        "49.54",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.288983333317",
        "0.29",
        "16.72",
        "32.82",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.191449999989",
        "0.19",
        "16.82",
        "16.00",
    );
    check_ip(
        &model,
        5,
        0,
        "16.09",
        "0.005833333333",
        "0.0933333333279",
        "0.09",
        "16.00",
        "0.00",
    );
}

/// `test_interestPauseBorderedByPeriod_Amt100_*`: pause Feb 1 – Mar 1 covering exactly period 1 (both endpoints on
/// period boundaries) — period 1 accrues no interest (full EMI to principal), cascading the later balances.
#[test]
fn interest_pause_bordered_by_period() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::February, 1), d(2024, Month::March, 1));

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
        "0.005833333333",
        "0.0",
        "0.00",
        "17.01",
        "66.54",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.388149999977",
        "0.39",
        "16.62",
        "49.92",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.291199999983",
        "0.29",
        "16.72",
        "33.20",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.193666666656",
        "0.19",
        "16.82",
        "16.38",
    );
    check_ip(
        &model,
        5,
        0,
        "16.48",
        "0.005833333333",
        "0.0955499999946",
        "0.10",
        "16.38",
        "0.00",
    );
}

/// `test_interestPauseOnTheFirstAndThirdRepaymentPeriodAmt100_*`: two separate pauses (Jan 2 – Feb 1 and Mar 2 – Apr 1)
/// zero out periods 0 and 2 entirely — confirms multiple non-adjacent pauses compose correctly.
#[test]
fn interest_pause_first_and_third_period() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::January, 2), d(2024, Month::February, 1));
    model.apply_interest_pause(d(2024, Month::March, 2), d(2024, Month::April, 1));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.00", "0.00", "17.01", "82.99",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.00",
        "0.00",
        "17.01",
        "82.99",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.484108333307",
        "0.48",
        "16.53",
        "66.46",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.00",
        "0.00",
        "17.01",
        "49.45",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.288458333317",
        "0.29",
        "16.72",
        "32.73",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.190924999989",
        "0.19",
        "16.82",
        "15.91",
    );
    check_ip(
        &model,
        5,
        0,
        "16.00",
        "0.005833333333",
        "0.0928083333279",
        "0.09",
        "15.91",
        "0.00",
    );
}

/// `test_interestPauseOnTheFirstDayOfFirstPeriodAmt100_*`: pause Jan 2 – Jan 10 inside the first period — splits
/// period 0 into a stub, a paused segment, and a real segment carrying the reduced first-period interest (0.41).
#[test]
fn interest_pause_first_day_of_first_period() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.apply_interest_pause(d(2024, Month::January, 2), d(2024, Month::January, 10));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.41", "16.60", "83.40",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.001693548387",
        "0.0",
        "0.41",
        "16.60",
        "83.40",
    );
    check_ip(
        &model,
        0,
        2,
        "17.01",
        "0.004139784946",
        "0.413978494600",
        "0.41",
        "16.60",
        "83.40",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.486499999972",
        "0.49",
        "16.52",
        "66.88",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.390133333310",
        "0.39",
        "16.62",
        "50.26",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.293183333317",
        "0.29",
        "16.72",
        "33.54",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.195649999989",
        "0.20",
        "16.81",
        "16.73",
    );
    check_ip(
        &model,
        5,
        0,
        "16.83",
        "0.005833333333",
        "0.0975916666611",
        "0.10",
        "16.73",
        "0.00",
    );
}

/// `test_interestPauseAndExistedMultipleInterestPeriodsAmt100_*`: two same-rate `changeInterestRate` calls (Feb 11,
/// Feb 21) pre-split period 1 into multiple interest periods, then a pause Feb 5–15 overlaps them — confirms the
/// pause-split and rate-change-split machinery compose (period 1 ends with 5 interest periods).
#[test]
fn interest_pause_and_existed_multiple_interest_periods() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.change_interest_rate(d(2024, Month::February, 11), &bd("7.0"));
    model.change_interest_rate(d(2024, Month::February, 21), &bd("7.0"));
    model.apply_interest_pause(d(2024, Month::February, 5), d(2024, Month::February, 15));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.58", "16.43", "83.57",
    );
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
        "0.30",
        "16.71",
        "66.86",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.001206896552",
        "0.0",
        "0.30",
        "16.71",
        "66.86",
    );
    check_ip(
        &model,
        1,
        2,
        "17.01",
        "0.001005747126",
        "0.0",
        "0.30",
        "16.71",
        "66.86",
    );
    check_ip(
        &model,
        1,
        3,
        "17.01",
        "0.001005747126",
        "0.0840502873475",
        "0.30",
        "16.71",
        "66.86",
    );
    check_ip(
        &model,
        1,
        4,
        "17.01",
        "0.002011494253",
        "0.168100574723",
        "0.30",
        "16.71",
        "66.86",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.390016666645",
        "0.39",
        "16.62",
        "50.24",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.29306666665",
        "0.29",
        "16.72",
        "33.52",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.195533333322",
        "0.20",
        "16.81",
        "16.71",
    );
    check_ip(
        &model,
        5,
        0,
        "16.81",
        "0.005833333333",
        "0.0974749999944",
        "0.10",
        "16.71",
        "0.00",
    );
}

/// `test_interestPauseBetweenRepaymentPeriodsAndExistedMultipleInterestPeriods_*`: four same-rate `changeInterestRate`
/// calls (Feb 6/21, Mar 6/21) pre-split periods 1 and 2, then a pause Feb 10 – Mar 10 spans both — confirms the
/// composition across two repayment periods each carrying multiple interest periods.
#[test]
fn interest_pause_between_repayment_periods_and_existed_multiple() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.change_interest_rate(d(2024, Month::February, 6), &bd("7.0"));
    model.change_interest_rate(d(2024, Month::February, 21), &bd("7.0"));
    model.change_interest_rate(d(2024, Month::March, 6), &bd("7.0"));
    model.change_interest_rate(d(2024, Month::March, 21), &bd("7.0"));
    model.apply_interest_pause(d(2024, Month::February, 10), d(2024, Month::March, 10));

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.58", "16.43", "83.57",
    );
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
        "0.000804597701",
        "0.0672402298812",
        "0.13",
        "16.88",
        "66.69",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.000804597701",
        "0.0672402298824",
        "0.13",
        "16.88",
        "66.69",
    );
    check_ip(
        &model,
        1,
        2,
        "17.01",
        "0.002212643678",
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
        "0.000752688172",
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
        "0.000940860215",
        "0.0",
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

/// `test_interestPauseOnFirstDayOfMonthAndExistedMultipleInterestPeriods_*`: same-rate `changeInterestRate` on Feb 1
/// (a period boundary) and Feb 6, then a pause Feb 1–5 — the no-op rate changes collapse and the result matches the
/// plain firstDayOfMonth pause.
#[test]
fn interest_pause_first_day_of_month_and_existed_multiple() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.change_interest_rate(d(2024, Month::February, 1), &bd("7.0"));
    model.change_interest_rate(d(2024, Month::February, 6), &bd("7.0"));
    model.apply_interest_pause(d(2024, Month::February, 1), d(2024, Month::February, 5));

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
        "0.000804597701",
        "0.0",
        "0.42",
        "16.59",
        "66.96",
    );
    check_ip(
        &model,
        1,
        1,
        "17.01",
        "0.005028735632",
        "0.420150862055",
        "0.42",
        "16.59",
        "66.96",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.390599999978",
        "0.39",
        "16.62",
        "50.34",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.293649999983",
        "0.29",
        "16.72",
        "33.62",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.196116666655",
        "0.20",
        "16.81",
        "16.81",
    );
    check_ip(
        &model,
        5,
        0,
        "16.91",
        "0.005833333333",
        "0.0980583333276",
        "0.10",
        "16.81",
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

    // generation block — per-interest-period rateFactor + calculatedDueInterest (Java golden
    // ProgressiveEMICalculatorTest.java:4551-4557, including the (0,0) disbursement stub and the (0,1) real period).
    check_ip(
        &model, 0, 0, "17.43", "0.0", "0.0", "1.33", "16.10", "83.90",
    );
    check_ip(
        &model,
        0,
        1,
        "17.43",
        "0.013315561644",
        "1.3315561644",
        "1.33",
        "16.10",
        "83.90",
    );
    check_ip(
        &model,
        1,
        0,
        "17.43",
        "0.012456493151",
        "1.04509977537",
        "1.05",
        "16.38",
        "67.52",
    );
    check_ip(
        &model,
        2,
        0,
        "17.43",
        "0.013315561644",
        "0.899066722202",
        "0.90",
        "16.53",
        "50.99",
    );
    check_ip(
        &model,
        3,
        0,
        "17.43",
        "0.012886027397",
        "0.657058536972",
        "0.66",
        "16.77",
        "34.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.43",
        "0.013315561644",
        "0.455658519458",
        "0.46",
        "16.97",
        "17.25",
    );
    check_ip(
        &model,
        5,
        0,
        "17.47",
        "0.012886027397",
        "0.222283972598",
        "0.22",
        "17.25",
        "0.00",
    );

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

/// `test_transactionInMiddleOfPeriod_EQUAL_AMORTIZATION_FULL_INTEREST` (Java
/// ProgressiveEMICalculatorTest.java:4584-4665): faithful 1:1 port. 100 @ 15.678% DAYS_365/ACTUAL generates the
/// EMI-17.43 schedule, then a real payment sequence runs — on-time EMI on period 0, an MIR overpayment of period 1,
/// MIR payments of the last installment (period 5 in full + 2.57 of period 4) all dated Feb 1, plus two offsetting
/// balance corrections on Apr 1 / Apr 15 derived from `get_due_amounts` — before a FULL_INTEREST re-age on Apr 15 into
/// 6 installments. Invariant (Java:4660-4663): outstanding principal/interest till maturity is unchanged by the re-age.
/// Stresses calculate_paid_balances_after_date + create_repayment_period_for_early_repaid with non-zero paid amounts.
#[test]
fn reage_after_payments_preserves_outstanding() {
    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    // generation block — Java golden ProgressiveEMICalculatorTest.java:4610-4616.
    check_ip(
        &model, 0, 0, "17.43", "0.0", "0.0", "1.33", "16.10", "83.90",
    );
    check_ip(
        &model,
        0,
        1,
        "17.43",
        "0.013315561644",
        "1.3315561644",
        "1.33",
        "16.10",
        "83.90",
    );
    check_ip(
        &model,
        1,
        0,
        "17.43",
        "0.012456493151",
        "1.04509977537",
        "1.05",
        "16.38",
        "67.52",
    );
    check_ip(
        &model,
        2,
        0,
        "17.43",
        "0.013315561644",
        "0.899066722202",
        "0.90",
        "16.53",
        "50.99",
    );
    check_ip(
        &model,
        3,
        0,
        "17.43",
        "0.012886027397",
        "0.657058536972",
        "0.66",
        "16.77",
        "34.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.43",
        "0.013315561644",
        "0.455658519458",
        "0.46",
        "16.97",
        "17.25",
    );
    check_ip(
        &model,
        5,
        0,
        "17.47",
        "0.012886027397",
        "0.222283972598",
        "0.22",
        "17.25",
        "0.00",
    );

    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let apr1 = d(2024, Month::April, 1);
    let may1 = d(2024, Month::May, 1);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);

    // repay EMI on first period due date (Java:4619-4622)
    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));

    // repay EMI on first period due date - next installment for 2nd period (Java:4625-4626)
    model.pay_principal(feb1, mar1, feb1, &m("17.43"));

    // repay 20 for last installment on first period due date -- ie MIR (Java:4629-4632)
    model.pay_principal(jun1, jul1, feb1, &m("17.43"));
    model.pay_principal(may1, jun1, feb1, &m("2.57"));

    // two offsetting balance corrections derived from getDueAmounts (Java:4634-4642)
    let due_p = model
        .get_due_amounts(mar1, apr1, apr1)
        .unwrap()
        .due_principal;
    model.add_balance_correction(apr1, &due_p);
    let due_p2 = model
        .get_due_amounts(mar1, apr1, apr1)
        .unwrap()
        .due_principal;
    model.add_balance_correction(d(2024, Month::April, 15), &m("0").minus(&due_p2));

    let maturity_before = model.periods.last().unwrap().due;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(maturity_before);

    // re-age on Apr 15 into 6 installments from Apr 20 (Java:4644-4655)
    model.re_age_equal_amortization(
        d(2024, Month::April, 15),
        d(2024, Month::April, 20),
        6,
        1,
        true,
    );

    let maturity_after = model.periods.last().unwrap().due;
    let (p_after, i_after) = model.get_outstanding_amounts_till_date(maturity_after);
    // Java:4660-4663 — outstanding interest/principal unchanged by the re-age.
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "outstanding interest unchanged by re-age (after payments)"
    );
    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "outstanding principal unchanged by re-age (after payments)"
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

/// `test_chargeBackOn2ndRP_ReAgeingOn2stRPsDueDate` / `…ReAgeingDuring2stRP` (EQUAL_AMORTIZATION_FULL_INTEREST): same as
/// the chargeback+re-age invariant but re-aging on the 2nd period's due date (Mar 1) and mid-2nd-period (Feb 10). The
/// invariant (outstanding preserved) must hold for any re-age date.
#[test]
fn reage_after_chargeback_various_dates() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    for reage_date in [d(2024, Month::March, 1), d(2024, Month::February, 10)] {
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

        let (p_before, i_before) =
            model.get_outstanding_amounts_till_date(model.periods.last().unwrap().due);
        model.re_age_equal_amortization(reage_date, reage_date, 6, 1, true);
        let (p_after, i_after) =
            model.get_outstanding_amounts_till_date(model.periods.last().unwrap().due);

        assert_eq!(
            p_after.to_plain_string(),
            p_before.to_plain_string(),
            "re-age on {reage_date} preserves outstanding principal"
        );
        assert_eq!(
            i_after.to_plain_string(),
            i_before.to_plain_string(),
            "re-age on {reage_date} preserves outstanding interest"
        );
    }
}

/// `test_chargeback_transactionInMiddleOfPeriod_EQUAL_EQUAL_AMORTIZATION_PAYABLE_INTEREST`: 100 @ 7% DAYS_360/30 —
/// repay period 0, mid-period chargeback (16.43 principal + 0.58 interest) on Mar 15, then a PAYABLE_INTEREST re-age on
/// Mar 15 (start Apr 1). Outstanding-till-transaction-date before == outstanding-till-maturity after.
#[test]
fn reage_payable_after_mid_period_chargeback() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar15 = d(2024, Month::March, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.credit_principal(mar15, &m("16.43"));
    model.credit_interest(mar15, &m("0.58"));

    let (p_before, i_before) = model.get_outstanding_amounts_till_date(mar15);
    model.re_age_equal_amortization(mar15, d(2024, Month::April, 1), 6, 1, false);
    let (p_after, i_after) =
        model.get_outstanding_amounts_till_date(model.periods.last().unwrap().due);

    assert_eq!(
        p_after.to_plain_string(),
        p_before.to_plain_string(),
        "PAYABLE re-age after chargeback preserves outstanding principal"
    );
    assert_eq!(
        i_after.to_plain_string(),
        i_before.to_plain_string(),
        "PAYABLE re-age after chargeback preserves outstanding interest"
    );
}

/// `test_reschedule_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month`: changeDueDate moves the period
/// containing Feb 1 (= period 0, first-inclusive) to due Mar 1, extending period 0 to 2 months and shifting periods 1-4
/// forward by a month; the last period (period 5) is extended to Jul1-2024 → 2026-06-01 via the fixedLength override
/// (`start + (fixedLength=0 + variationDays=29) months`), so its interest grows to 2.35 via the 23-month periodRatio.
#[test]
fn change_due_date_reschedule() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    model.change_due_date(
        d(2024, Month::February, 1),
        d(2024, Month::March, 1),
        Some(0),
    );

    // period 0 extended to 2 months (interest 1.17); its real interest period.
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "1.1666666667",
        "1.17",
        "15.84",
        "84.16",
    );
    check_rp(&model, 1, "17.01", "0.49", "16.52", "67.64");
    check_rp(&model, 2, "17.01", "0.39", "16.62", "51.02");
    check_rp(&model, 3, "17.01", "0.30", "16.71", "34.31");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "17.50");
    // period 5 extended ~23 months → interest 2.35, EMI 19.85.
    check_rp(&model, 5, "19.85", "2.35", "17.50", "0.00");
    assert_eq!(
        model.periods[5].due,
        d(2026, Month::June, 1),
        "rp5 due extended"
    );
}

/// `test_two_reschedules_disbursedAmt100_*`: two changeDueDate calls — move period 0 to Mar 1, then the period
/// containing May 1 to Jun 1. The cumulative reschedules extend the last period further (interest 2.51, EMI 20.41).
#[test]
fn change_due_date_two_reschedules() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    model.change_due_date(
        d(2024, Month::February, 1),
        d(2024, Month::March, 1),
        Some(0),
    );
    model.change_due_date(d(2024, Month::May, 1), d(2024, Month::June, 1), Some(0));

    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "1.1666666667",
        "1.17",
        "15.84",
        "84.16",
    );
    check_rp(&model, 1, "17.01", "0.49", "16.52", "67.64");
    check_rp(&model, 2, "17.01", "0.79", "16.22", "51.42");
    check_rp(&model, 3, "17.01", "0.30", "16.71", "34.71");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "17.90");
    check_rp(&model, 5, "20.41", "2.51", "17.90", "0.00");
}

/// `test_reschedule_partial_period_disbursedAmt100_*`: changeDueDate moves the period containing Mar 1 (= period 1) to a
/// NON-month-aligned due date Apr 15, extending period 1 to a partial ~2.5-month period (interest 1.20 via a fractional
/// periodRatio ≈ 2.467). Exercises calculate_period_ratio on a non-month-aligned period.
#[test]
fn change_due_date_partial_period() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    model.change_due_date(d(2024, Month::March, 1), d(2024, Month::April, 15), Some(0));

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
    check_rp(&model, 1, "17.01", "1.20", "15.81", "67.76");
    check_rp(&model, 2, "17.01", "0.40", "16.61", "51.15");
    check_rp(&model, 3, "17.01", "0.30", "16.71", "34.44");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "17.63");
    check_rp(&model, 5, "21.59", "3.96", "17.63", "0.00");
}

/// `testFlatDaily_1_Month_360_30` (first two disbursement blocks): FLAT interest method, DAYS_360/30, 12%, 4 monthly
/// periods. Disburse 750 → EMI 195.00 (flat interest 7.50/period on the original principal, equal principal 187.50);
/// then a second disbursement of 250 (same date) → EMI 260.00 (flat interest 10.00 on 1000). Exercises the FLAT
/// interest method in recalc (flat ip interest on total disbursed, flat EMI = (totalDisbursed + Σinterest)/n).
#[test]
fn flat_recalc_multi_disbursement() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
    ];
    let jan1 = d(2024, Month::January, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("750.0"));
    check_rp(&model, 0, "195.00", "7.50", "187.50", "562.50");
    check_rp(&model, 1, "195.00", "7.50", "187.50", "375.00");
    check_rp(&model, 2, "195.00", "7.50", "187.50", "187.50");
    check_rp(&model, 3, "195.00", "7.50", "187.50", "0.00");

    model.add_disbursement(jan1, &m("250.0"));
    check_rp(&model, 0, "260.00", "10.00", "250.00", "750.00");
    check_rp(&model, 1, "260.00", "10.00", "250.00", "500.00");
    check_rp(&model, 2, "260.00", "10.00", "250.00", "250.00");
    check_rp(&model, 3, "260.00", "10.00", "250.00", "0.00");
}

/// `testExtraTermsFlat`: 1000 @ 12% FLAT (sameAsRepayment → flat interest 10/period; modeled here via DAYS_360/30 which
/// gives the same 0.01 rate factor). After generation (EMI 343.33 over 3 periods), addRepaymentPeriods extends the
/// schedule and re-amortizes the remaining balance over the new period set (EMI 232.22 over 3, then 121.11 over 4).
#[test]
fn extra_terms_flat_add_repayment_periods() {
    let periods = [
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
        (d(2024, Month::July, 1), d(2024, Month::August, 1)),
        (d(2024, Month::August, 1), d(2024, Month::September, 1)),
    ];
    let jun1 = d(2024, Month::June, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );
    model.add_disbursement(jun1, &m("1000.0"));
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67");
    check_rp(&model, 1, "343.33", "10.00", "333.33", "333.34");
    check_rp(&model, 2, "343.34", "10.00", "333.34", "0.00");

    // add 1 extra repayment period → re-amortize periods 1..3.
    model.add_repayment_periods(d(2024, Month::July, 2), 1);
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67");
    check_rp(&model, 1, "232.22", "10.00", "222.22", "444.45");
    check_rp(&model, 2, "232.22", "10.00", "222.22", "222.23");
    check_rp(&model, 3, "232.23", "10.00", "222.23", "0.00");

    // add 2 more → re-amortize periods 2..5.
    model.add_repayment_periods(d(2024, Month::August, 2), 2);
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67");
    check_rp(&model, 1, "232.22", "10.00", "222.22", "444.45");
    check_rp(&model, 2, "121.11", "10.00", "111.11", "333.34");
    check_rp(&model, 3, "121.11", "10.00", "111.11", "222.23");
    check_rp(&model, 4, "121.11", "10.00", "111.11", "111.12");
    check_rp(&model, 5, "121.12", "10.00", "111.12", "0.00");
}

/// `test_dailyInterest_disbursedAmt2000_*` (checkPeriod block): single-period declining loan, 1000 disbursed Jan 1 then
/// 1000 mid-period on Jan 15 — the 2nd disbursement splits period 0 into Jan1-Jan15 (1000) and Jan15-Feb1 (2000). EMI
/// 2009.03, interest 9.03 (5.83 on the first 1000 for 31 days + 3.20 on the second from Jan 15). Mid-period declining
/// multi-disbursement via the new-model merge path.
#[test]
fn declining_mid_period_second_disbursement() {
    let periods = [(d(2024, Month::January, 1), d(2024, Month::February, 1))];
    let jan1 = d(2024, Month::January, 1);
    let jan15 = d(2024, Month::January, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("7.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    model.add_disbursement(jan1, &m("1000.0"));
    model.add_disbursement(jan15, &m("1000.0"));

    check_ip(
        &model,
        0,
        1,
        "2009.03",
        "0.002634408602",
        "2.634408602",
        "9.03",
        "2000.00",
        "0.00",
    );
    check_ip(
        &model,
        0,
        2,
        "2009.03",
        "0.003198924731",
        "6.397849462",
        "9.03",
        "2000.00",
        "0.00",
    );
}

/// `testFlatDaily_1_Month_360_30` (third disbursement block): after 1000 disbursed (EMI 260), a mid-period FLAT
/// disbursement of 250 on Feb 6 splits period 1 and re-amortizes — period 0 stays 260, periods 1-3 become 345.69 with
/// period 1 carrying the partial mid-period interest (12.07).
#[test]
fn flat_mid_period_third_disbursement() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
    ];
    let jan1 = d(2024, Month::January, 1);
    let feb6 = d(2024, Month::February, 6);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );
    model.add_disbursement(jan1, &m("750.0"));
    model.add_disbursement(jan1, &m("250.0"));
    model.add_disbursement(feb6, &m("250.0"));

    check_rp(&model, 0, "260.00", "10.00", "250.00", "750.00");
    check_rp(&model, 1, "345.69", "12.07", "333.62", "666.38");
    check_rp(&model, 2, "345.69", "12.50", "333.19", "333.19");
    check_rp(&model, 3, "345.69", "12.50", "333.19", "0.00");
}

/// `testExtraTermsDecliningBalanceWithFullyRepaidPeriods`: 1000 @ 12% DAYS_360/30, 4 periods. Pay the last period (MIR)
/// and the first period early, then addRepaymentPeriods around the fully-paid periods — paid periods 0 and 3 stay
/// fixed while the remaining balance re-amortizes across the unpaid + new periods.
#[test]
fn extra_terms_declining_with_fully_repaid_periods() {
    let periods = [
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
        (d(2024, Month::July, 1), d(2024, Month::August, 1)),
        (d(2024, Month::August, 1), d(2024, Month::September, 1)),
        (d(2024, Month::September, 1), d(2024, Month::October, 1)),
    ];
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let sep1 = d(2024, Month::September, 1);
    let oct1 = d(2024, Month::October, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    model.add_disbursement(jun1, &m("1000.0"));
    check_rp(&model, 0, "256.28", "10.00", "246.28", "753.72");
    check_rp(&model, 3, "256.29", "2.54", "253.75", "0.00");

    model.pay_principal(sep1, oct1, jun1, &m("256.29"));
    model.pay_principal(jun1, jul1, jun1, &m("256.28"));
    check_rp(&model, 0, "256.28", "0.00", "256.28", "487.43");
    assert!(model.period_fully_paid(0));
    check_rp(&model, 1, "256.28", "9.74", "246.54", "240.89");
    check_rp(&model, 2, "243.30", "2.41", "240.89", "0.00");
    check_rp(&model, 3, "256.29", "0.00", "256.29", "0.00");
    assert!(model.period_fully_paid(3));

    model.add_repayment_periods(d(2024, Month::June, 15), 1);
    check_rp(&model, 1, "206.04", "9.74", "196.30", "291.13");
    check_rp(&model, 4, "89.76", "1.76", "88.00", "0.00");

    model.add_repayment_periods(d(2024, Month::June, 15), 2);
    check_rp(&model, 1, "126.91", "9.74", "117.17", "370.26");
    check_rp(&model, 5, "126.33", "1.25", "125.08", "0.00");
    check_rp(&model, 6, "0.00", "0.00", "0.00", "0.00");
}

/// `testExtraTermsFlatWithFullyRepaidPeriods`: FLAT, 1000 @ 12% DAYS_360/30, 4 periods, EMI 260 (250 principal + 10 flat
/// interest each). Pay the last period (MIR) and the first period in full (principal + interest), then addRepaymentPeriods
/// around the fully-paid periods — the flat reallocation branch redistributes remaining principal across unpaid + new
/// periods while paid periods 0 and 3 stay fixed, and a trailing interest-only stub period appears.
#[test]
fn extra_terms_flat_with_fully_repaid_periods() {
    let periods = [
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
        (d(2024, Month::July, 1), d(2024, Month::August, 1)),
        (d(2024, Month::August, 1), d(2024, Month::September, 1)),
        (d(2024, Month::September, 1), d(2024, Month::October, 1)),
    ];
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let sep1 = d(2024, Month::September, 1);
    let oct1 = d(2024, Month::October, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );
    model.add_disbursement(jun1, &m("1000.0"));
    check_rp(&model, 0, "260.00", "10.00", "250.00", "750.00");
    check_rp(&model, 3, "260.00", "10.00", "250.00", "0.00");

    // merchant refund on the last (fully repaid) installment, then early full repayment of period 0
    model.pay_principal(sep1, oct1, jun1, &m("250.0"));
    model.pay_interest(sep1, oct1, jun1, &m("10.0"));
    check_rp(&model, 3, "260.00", "10.00", "250.00", "0.00");
    assert!(model.period_fully_paid(3));

    model.pay_principal(jun1, jul1, jun1, &m("250.0"));
    model.pay_interest(jun1, jul1, jun1, &m("10.0"));
    check_rp(&model, 0, "260.00", "10.00", "250.00", "500.00");
    assert!(model.period_fully_paid(0));
    check_rp(&model, 1, "260.00", "10.00", "250.00", "250.00");

    model.add_repayment_periods(d(2024, Month::June, 15), 1);
    check_rp(&model, 1, "210.00", "10.00", "200.00", "300.00");
    check_rp(&model, 2, "210.00", "10.00", "200.00", "100.00");
    check_rp(&model, 4, "110.00", "10.00", "100.00", "0.00");

    model.add_repayment_periods(d(2024, Month::June, 15), 2);
    check_rp(&model, 1, "152.86", "10.00", "142.86", "357.14");
    check_rp(&model, 4, "152.86", "10.00", "142.86", "71.42");
    check_rp(&model, 5, "81.42", "10.00", "71.42", "0.00");
    check_rp(&model, 6, "10.00", "10.00", "0.00", "0.00");
}

/// `test_payoff_on0115_*`: 100 @ 7% DAYS_360/30, payoff inside the FIRST period on Jan 15 (before its Feb 1 due date) —
/// getDueAmounts forecasts the prorated payoff (16.75/0.26), then every period is paid early on Jan 15. Mirrors the
/// on0215 payoff but with the payoff date in period 0.
#[test]
fn payoff_on_jan15() {
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
    let jan15 = d(2024, Month::January, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let due_on_due = model.get_due_amounts(jan1, feb1, feb1).unwrap();
    assert_eq!(due_on_due.due_principal.to_plain_string(), "16.43");
    assert_eq!(due_on_due.due_interest.to_plain_string(), "0.58");
    let due_payoff = model.get_due_amounts(jan1, feb1, jan15).unwrap();
    assert_eq!(due_payoff.due_principal.to_plain_string(), "16.75");
    assert_eq!(due_payoff.due_interest.to_plain_string(), "0.26");

    model.pay_principal(jan1, feb1, jan15, &m("16.75"));
    model.pay_interest(jan1, feb1, jan15, &m("0.26"));
    model.pay_principal(feb1, mar1, jan15, &m("17.01"));
    model.pay_principal(mar1, apr1, jan15, &m("17.01"));
    model.pay_principal(apr1, may1, jan15, &m("17.01"));
    model.pay_principal(may1, jun1, jan15, &m("17.01"));

    let due_last = model.get_due_amounts(jun1, jul1, jul1).unwrap();
    assert_eq!(due_last.due_principal.to_plain_string(), "15.21");
    assert_eq!(due_last.due_interest.to_plain_string(), "0.50");

    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.26", "16.75", "15.21",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.002634408602",
        "0.2634408602",
        "0.26",
        "16.75",
        "15.21",
    );
    check_ip(
        &model,
        0,
        2,
        "17.01",
        "0.003198924731",
        "0.0486556451585",
        "0.26",
        "16.75",
        "15.21",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.0887249999948",
        "0.00",
        "17.01",
        "15.21",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.0887249999949",
        "0.00",
        "17.01",
        "15.21",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.0887249999949",
        "0.00",
        "17.01",
        "15.21",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.0887249999949",
        "0.00",
        "17.01",
        "15.21",
    );
    check_ip(
        &model,
        5,
        0,
        "15.71",
        "0.005833333333",
        "0.0887249999949",
        "0.50",
        "15.21",
        "0.00",
    );

    model.pay_principal(jun1, jul1, jan15, &m("15.21"));

    check_ip(&model, 0, 0, "17.01", "0.0", "0.0", "0.26", "16.75", "0.00");
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.002634408602",
        "0.2634408602",
        "0.26",
        "16.75",
        "0.00",
    );
    check_ip(
        &model,
        0,
        2,
        "17.01",
        "0.003198924731",
        "0.0",
        "0.26",
        "16.75",
        "0.00",
    );
    check_ip(
        &model,
        1,
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
        "15.21",
        "0.005833333333",
        "0.0",
        "0.00",
        "15.21",
        "0.00",
    );
}

/// `test_leap_year_only_actual_no_effect_on_360_loan`: a 100 @ 7% DAYS_360/30 loan WITH the FEB_29_PERIOD_ONLY custom
/// strategy set — the strategy is correctly ignored for 360-day loans, so the schedule equals the standard 360/30 one.
#[test]
fn leap_year_no_effect_on_360() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.58", "16.43", "83.57",
    );
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
        "67.05",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.391124999979",
        "0.39",
        "16.62",
        "50.43",
    );
    check_ip(
        &model,
        3,
        0,
        "17.01",
        "0.005833333333",
        "0.294174999983",
        "0.29",
        "16.72",
        "33.71",
    );
    check_ip(
        &model,
        4,
        0,
        "17.01",
        "0.005833333333",
        "0.196641666655",
        "0.20",
        "16.81",
        "16.90",
    );
    check_ip(
        &model,
        5,
        0,
        "17.00",
        "0.005833333333",
        "0.0985833333276",
        "0.10",
        "16.90",
        "0.00",
    );
}

/// `test_chargeback_less_principal_*`: repay periods 0 and 1, then a chargeback (creditPrincipal 14.51 +
/// creditInterest 0.49) on Mar 1 — the credited principal lands on period 2 (duePrincipal jumps to 31.04) and cascades.
#[test]
fn chargeback_less_principal() {
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
    model.credit_principal(mar1, &m("14.51"));
    model.credit_interest(mar1, &m("0.49"));

    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.005833333333",
        "0.487491666639",
        "0.49",
        "16.52",
        "81.56",
    );
    check_ip(
        &model,
        2,
        0,
        "17.01",
        "0.005833333333",
        "0.96576666664",
        "0.97",
        "31.04",
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
}

/// `test_chargeback_less_principal_and_no_chargeback_interest_*`: as above but the chargeback credits principal only
/// (creditPrincipal 15.0, no interest) — period 2 duePrincipal 31.53, interest unchanged.
#[test]
fn chargeback_less_principal_no_interest() {
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
}

/// `test_multi_disbursedAmt200_2ndOnDueDate_*`: 100 @ 9.4822% DAYS_360/30, then a 2nd 100 disbursement ON period 2's
/// start date (Mar 1) — the merge path re-amortizes from Mar 1 onward (EMI 42.63), periods 0-1 unchanged.
#[test]
fn multi_disbursed_200_2nd_on_due_date() {
    let mut model = new_empty(
        &six_monthly(),
        &bd("9.4822"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    model.add_disbursement(d(2024, Month::January, 1), &m("100.0"));
    check_ip(
        &model,
        0,
        1,
        "17.13",
        "0.007901833333",
        "0.790183333301",
        "0.79",
        "16.34",
        "83.66",
    );
    check_ip(
        &model,
        5,
        0,
        "17.13",
        "0.007901833333",
        "0.134331166661",
        "0.13",
        "17.00",
        "0.00",
    );

    model.add_disbursement(d(2024, Month::March, 1), &m("100.0"));
    check_ip(
        &model,
        1,
        0,
        "17.13",
        "0.007901833333",
        "0.66106737664",
        "0.66",
        "16.47",
        "167.19",
    );
    check_ip(
        &model,
        2,
        0,
        "42.63",
        "0.007901833333",
        "1.32110751494",
        "1.32",
        "41.31",
        "125.88",
    );
    check_ip(
        &model,
        3,
        0,
        "42.63",
        "0.007901833333",
        "0.994682779959",
        "0.99",
        "41.64",
        "84.24",
    );
    check_ip(
        &model,
        4,
        0,
        "42.63",
        "0.007901833333",
        "0.665650439972",
        "0.67",
        "41.96",
        "42.28",
    );
    check_ip(
        &model,
        5,
        0,
        "42.61",
        "0.007901833333",
        "0.334089513318",
        "0.33",
        "42.28",
        "0.00",
    );
}

/// `test_multiDisbursedAmt300InSamePeriod_*`: 100 @ 9.4822% on Jan 1 then 200 on Jan 8 (mid first period) — period 0
/// splits into stub + Jan1-Jan8 + Jan8-Feb1, EMI 51.33.
#[test]
fn multi_disbursed_300_in_same_period() {
    let mut model = new_empty(
        &six_monthly(),
        &bd("9.4822"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    model.add_disbursement(d(2024, Month::January, 1), &m("100"));
    check_ip(
        &model,
        5,
        0,
        "17.13",
        "0.007901833333",
        "0.134331166661",
        "0.13",
        "17.00",
        "0.00",
    );

    model.add_disbursement(d(2024, Month::January, 8), &m("200.0"));
    check_ip(
        &model, 0, 0, "51.33", "0.0", "0.0", "2.01", "49.32", "250.68",
    );
    check_ip(
        &model,
        0,
        1,
        "51.33",
        "0.001784284946",
        "0.178428494616",
        "2.01",
        "49.32",
        "250.68",
    );
    check_ip(
        &model,
        0,
        2,
        "51.33",
        "0.006117548387",
        "1.8352645161",
        "2.01",
        "49.32",
        "250.68",
    );
    check_ip(
        &model,
        1,
        0,
        "51.33",
        "0.007901833333",
        "1.98083157992",
        "1.98",
        "49.35",
        "201.33",
    );
    check_ip(
        &model,
        2,
        0,
        "51.33",
        "0.007901833333",
        "1.59087610493",
        "1.59",
        "49.74",
        "151.59",
    );
    check_ip(
        &model,
        3,
        0,
        "51.33",
        "0.007901833333",
        "1.19783891495",
        "1.20",
        "50.13",
        "101.46",
    );
    check_ip(
        &model,
        4,
        0,
        "51.33",
        "0.007901833333",
        "0.801720009967",
        "0.80",
        "50.53",
        "50.93",
    );
    check_ip(
        &model,
        5,
        0,
        "51.33",
        "0.007901833333",
        "0.40244037165",
        "0.40",
        "50.93",
        "0.00",
    );
}

/// `test_multiDisbursedAmt200InDifferentPeriod_*`: 100 @ 9.4822% on Jan 1 then 100 on Feb 15 (mid period 1) — period 1
/// splits at Feb 15, EMI 37.53.
#[test]
fn multi_disbursed_200_in_different_period() {
    let mut model = new_empty(
        &six_monthly(),
        &bd("9.4822"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    model.add_disbursement(d(2024, Month::January, 1), &m("100.0"));
    check_ip(
        &model,
        5,
        0,
        "17.13",
        "0.007901833333",
        "0.134331166661",
        "0.13",
        "17.00",
        "0.00",
    );

    model.add_disbursement(d(2024, Month::February, 15), &m("100.0"));
    check_ip(
        &model,
        1,
        0,
        "37.53",
        "0.003814678161",
        "0.31913597493",
        "1.07",
        "36.46",
        "147.20",
    );
    check_ip(
        &model,
        1,
        1,
        "37.53",
        "0.004087155172",
        "0.75064691889",
        "1.07",
        "36.46",
        "147.20",
    );
    check_ip(
        &model,
        2,
        0,
        "37.53",
        "0.007901833333",
        "1.16314986662",
        "1.16",
        "36.37",
        "110.83",
    );
    check_ip(
        &model,
        3,
        0,
        "37.53",
        "0.007901833333",
        "0.875760188295",
        "0.88",
        "36.65",
        "74.18",
    );
    check_ip(
        &model,
        4,
        0,
        "37.53",
        "0.007901833333",
        "0.586157996641",
        "0.59",
        "36.94",
        "37.24",
    );
    check_ip(
        &model,
        5,
        0,
        "37.53",
        "0.007901833333",
        "0.294264273321",
        "0.29",
        "37.24",
        "0.00",
    );
}

/// `test_disbursedAmt100_dayInYears364_daysInMonthActual_repayEvery2Week`: 100 @ 9.4822% DAYS_364, three 14-day
/// periods from 2024-01-01 — rate factor 9.4822%·14/364 = 0.003647, EMI 33.57.
#[test]
fn disbursed_364_repay_every_2_week() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 15)),
        (d(2024, Month::January, 15), d(2024, Month::January, 29)),
        (d(2024, Month::January, 29), d(2024, Month::February, 12)),
    ];
    let model = new_declining_balance_with_day_count(
        &periods,
        &bd("100"),
        &bd("9.4822"),
        DayCount::ActualFixedYear { days_in_year: 364 },
        2,
        RoundingMode::HalfEven,
    );
    check_ip(
        &model, 0, 0, "33.57", "0.0", "0.0", "0.36", "33.21", "66.79",
    );
    check_ip(
        &model,
        0,
        1,
        "33.57",
        "0.003647000000",
        "0.3647",
        "0.36",
        "33.21",
        "66.79",
    );
    check_ip(
        &model,
        1,
        0,
        "33.57",
        "0.003647000000",
        "0.24358313",
        "0.24",
        "33.33",
        "33.46",
    );
    check_ip(
        &model,
        2,
        0,
        "33.58",
        "0.003647000000",
        "0.12202862",
        "0.12",
        "33.46",
        "0.00",
    );
}

/// `test_reschedule_interest_on0201_4per_*`: 100 @ 7% DAYS_360/30, change rate to 4% effective Feb 2 (no payments) —
/// EMI drops to 16.88 from period 1 onward.
#[test]
fn reschedule_interest_on_feb2_clean() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    model.change_interest_rate(d(2024, Month::February, 2), &bd("4.0"));
    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.58", "16.43", "83.57",
    );
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
        "16.88",
        "0.003333333333",
        "0.278566666639",
        "0.28",
        "16.60",
        "66.97",
    );
    check_ip(
        &model,
        2,
        0,
        "16.88",
        "0.003333333333",
        "0.223233333311",
        "0.22",
        "16.66",
        "50.31",
    );
    check_ip(
        &model,
        3,
        0,
        "16.88",
        "0.003333333333",
        "0.167699999983",
        "0.17",
        "16.71",
        "33.60",
    );
    check_ip(
        &model,
        4,
        0,
        "16.88",
        "0.003333333333",
        "0.111999999989",
        "0.11",
        "16.77",
        "16.83",
    );
    check_ip(
        &model,
        5,
        0,
        "16.89",
        "0.003333333333",
        "0.0560999999943",
        "0.06",
        "16.83",
        "0.00",
    );
}

/// `test_reschedule_interest_on0201_2nd_EMI_not_changeable_*`: pay period 0 (interest+principal) and period 1 principal
/// (16.90) on Feb 1, then change rate to 4% effective Feb 2 — the prepaid period 1 EMI is not lowered (stays 17.01).
#[test]
fn reschedule_interest_on_feb2_with_prepaid() {
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
    model.pay_interest(jan1, feb1, feb1, &m("0.58"));
    model.pay_principal(jan1, feb1, feb1, &m("16.43"));
    model.pay_principal(feb1, mar1, feb1, &m("16.90"));
    model.change_interest_rate(d(2024, Month::February, 2), &bd("4.0"));
    check_ip(
        &model, 0, 0, "17.01", "0.0", "0.0", "0.58", "16.43", "66.67",
    );
    check_ip(
        &model,
        0,
        1,
        "17.01",
        "0.005833333333",
        "0.5833333333",
        "0.58",
        "16.43",
        "66.67",
    );
    check_ip(
        &model,
        1,
        0,
        "17.01",
        "0.003333333333",
        "0.222233333311",
        "0.00",
        "17.01",
        "66.56",
    );
    check_ip(
        &model,
        2,
        0,
        "16.83",
        "0.003333333333",
        "0.221866666644",
        "0.44",
        "16.39",
        "50.17",
    );
    check_ip(
        &model,
        3,
        0,
        "16.83",
        "0.003333333333",
        "0.167233333317",
        "0.17",
        "16.66",
        "33.51",
    );
    check_ip(
        &model,
        4,
        0,
        "16.83",
        "0.003333333333",
        "0.111699999989",
        "0.11",
        "16.72",
        "16.79",
    );
    check_ip(
        &model,
        5,
        0,
        "16.85",
        "0.003333333333",
        "0.0559666666611",
        "0.06",
        "16.79",
        "0.00",
    );
}

/// `ProgressiveEMICalculatorTest.testFlatDaily_firstRepaymentPeriodWithoutPrevious_doesNotThrowOnDisbursement`
/// (Java line 3424, nested class `InterestTypeFlatAndCalculationPeriodDaily` whose @BeforeEach (Java line 3412) sets
/// InterestMethod.FLAT, InterestCalculationPeriodMethod.DAILY, allowPartialPeriodInterestCalculation=true; the test
/// itself sets 12%, ACTUAL/ACTUAL day counts, 4 monthly periods, disburse 700.0). The Java test only asserts
/// addDisbursement does not throw on a schedule whose first repayment period has no previous period. We additionally
/// assert the first-period EMI equals the golden 181.94 from the sibling `testFlatDaily_1_Month_Actual_Actual`
/// (Java line 3463), which uses the identical settings and disbursement.
#[test]
fn flat_daily_first_period_without_previous_does_not_throw_on_disbursement() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
    ];
    let jan1 = d(2024, Month::January, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_empty(
        &periods,
        &bd("12.0"),
        DayCount::ActualActual {
            recognition_on_disbursement: false,
        },
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    // The point of the Java test: this must not panic on a first period that has no previous period.
    model.add_disbursement(jan1, &m("700.0"));

    // Golden first-period values from testFlatDaily_1_Month_Actual_Actual (Java line 3463): EMI 181.94.
    check_rp(&model, 0, "181.94", "7.11", "174.83", "525.17");
}

/// `ProgressiveEMICalculatorTest.test_generateInterestScheduleModel` (Java line 187): a pure structural
/// generation check. The Java assertions are: model non-null; loanProductRelatedDetail non-null;
/// installmentAmountInMultiplesOf == null; repaymentPeriods non-null; repaymentPeriods().size() == 4;
/// getLoanTermInDays() == 121 (2024-01-01 .. 2024-05-01 = 31+29+31+30 days). In Rust the model carries the
/// generated periods directly (no separate "schedule model" object), so the reproducible parts are the period
/// count and the loan-term span. There is no getLoanTermInDays() getter on `Model`, so the term is checked here
/// directly from the period endpoints.
#[test]
fn generate_interest_schedule_model_structural() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
    ];
    // No disbursement applied (Java generatePeriodInterestScheduleModel only builds the period structure).
    let model = new_empty(
        &periods,
        &bd("0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );

    // repaymentPeriods().size() == 4 (Java line 205).
    assert_eq!(model.periods.len(), 4);
    // getLoanTermInDays() == 121 (Java line 206): span from first period start to last period due.
    let term_days =
        (model.periods.last().unwrap().due - model.periods.first().unwrap().from).whole_days();
    assert_eq!(term_days, 121);
}

/// `test_transactionInMiddleOfPeriod_stringOnNextDueDate_EQUAL_EQUAL_AMORTIZATION_PAYABLE_INTEREST`
/// (ProgressiveEMICalculatorTest.java lines 4869-4950): 100 @ 15.678% DAYS_365/ACTUAL, six monthly periods.
/// On-time period 0, MIR overpayments of periods 1/5/4 booked on Feb 1, two balance corrections (apply then negate
/// the Mar->Apr due principal via getDueAmounts) on Apr 1 / Apr 15. Re-age (PAYABLE_INTEREST, full_interest=false)
/// with transaction date Apr 15 and re-age start May 1. Invariant: outstanding-till-transaction-date before the
/// re-age == outstanding-till-maturity after it. Generation block mirrors checkPeriod (java 4896-4902).
#[test]
fn reage_payable_mid_period_string_on_next_due_date() {
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

    // Generation block — java checkPeriod lines 4896-4902 (7-arg ip form: interestDue passed as calc & cumulated).
    check_ip(
        &model, 0, 0, "17.43", "0.0", "0.0", "1.33", "16.10", "83.90",
    );
    check_ip(
        &model,
        0,
        1,
        "17.43",
        "0.013315561644",
        "1.3315561644",
        "1.33",
        "16.10",
        "83.90",
    );
    check_ip(
        &model,
        1,
        0,
        "17.43",
        "0.012456493151",
        "1.04509977537",
        "1.05",
        "16.38",
        "67.52",
    );
    check_ip(
        &model,
        2,
        0,
        "17.43",
        "0.013315561644",
        "0.899066722202",
        "0.90",
        "16.53",
        "50.99",
    );
    check_ip(
        &model,
        3,
        0,
        "17.43",
        "0.012886027397",
        "0.657058536972",
        "0.66",
        "16.77",
        "34.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.43",
        "0.013315561644",
        "0.455658519458",
        "0.46",
        "16.97",
        "17.25",
    );
    check_ip(
        &model,
        5,
        0,
        "17.47",
        "0.012886027397",
        "0.222283972598",
        "0.22",
        "17.25",
        "0.00",
    );

    // repay EMI on first period due date
    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));
    // repay EMI on first period due date - next installment for 2nd period
    model.pay_principal(feb1, mar1, feb1, &m("17.43"));
    // repay last/penultimate installments on first period due date -- ie MIR
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
    model.re_age_equal_amortization(apr15, may1, 6, 1, false);
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

/// `test_transactionOnMaturityDate_stringAfterMaturityDate_EQUAL_EQUAL_AMORTIZATION_PAYABLE_INTEREST`
/// (ProgressiveEMICalculatorTest.java lines 4952-5023): same 100 @ 15.678% DAYS_365 schedule and the same on-time +
/// MIR payment sequence, but NO balance corrections. Re-age (PAYABLE_INTEREST) with transaction date on the maturity
/// date (Jul 1) and re-age start after maturity (Jul 15). Invariant: outstanding-till-transaction-date before the
/// re-age == outstanding-till-maturity after it. Generation block mirrors checkPeriod (java 4979-4985).
#[test]
fn reage_payable_transaction_on_maturity_string_after_maturity() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let may1 = d(2024, Month::May, 1);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let jul15 = d(2024, Month::July, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );

    // Generation block — java checkPeriod lines 4979-4985.
    check_ip(
        &model, 0, 0, "17.43", "0.0", "0.0", "1.33", "16.10", "83.90",
    );
    check_ip(
        &model,
        0,
        1,
        "17.43",
        "0.013315561644",
        "1.3315561644",
        "1.33",
        "16.10",
        "83.90",
    );
    check_ip(
        &model,
        1,
        0,
        "17.43",
        "0.012456493151",
        "1.04509977537",
        "1.05",
        "16.38",
        "67.52",
    );
    check_ip(
        &model,
        2,
        0,
        "17.43",
        "0.013315561644",
        "0.899066722202",
        "0.90",
        "16.53",
        "50.99",
    );
    check_ip(
        &model,
        3,
        0,
        "17.43",
        "0.012886027397",
        "0.657058536972",
        "0.66",
        "16.77",
        "34.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.43",
        "0.013315561644",
        "0.455658519458",
        "0.46",
        "16.97",
        "17.25",
    );
    check_ip(
        &model,
        5,
        0,
        "17.47",
        "0.012886027397",
        "0.222283972598",
        "0.22",
        "17.25",
        "0.00",
    );

    // repay EMI on first period due date
    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));
    // repay EMI on first period due date - next installment for 2nd period
    model.pay_principal(feb1, mar1, feb1, &m("17.43"));
    // repay last/penultimate installments on first period due date -- ie MIR
    model.pay_principal(jun1, jul1, feb1, &m("17.43"));
    model.pay_principal(may1, jun1, feb1, &m("2.57"));

    let transaction_date = jul1;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(transaction_date);
    model.re_age_equal_amortization(transaction_date, jul15, 6, 1, false);
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

/// `test_transactionAfterMaturityDate_EQUAL_EQUAL_AMORTIZATION_PAYABLE_INTEREST`
/// (ProgressiveEMICalculatorTest.java lines 5025-5096): same 100 @ 15.678% DAYS_365 schedule + on-time/MIR payment
/// sequence, NO balance corrections. Re-age (PAYABLE_INTEREST) with transaction date AFTER maturity (Jul 10) and
/// re-age start Jul 15. Invariant: outstanding-till-transaction-date before == outstanding-till-maturity after.
/// Generation block mirrors checkPeriod (java 5052-5058).
#[test]
fn reage_payable_transaction_after_maturity() {
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let may1 = d(2024, Month::May, 1);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let jul10 = d(2024, Month::July, 10);
    let jul15 = d(2024, Month::July, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));

    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );

    // Generation block — java checkPeriod lines 5052-5058.
    check_ip(
        &model, 0, 0, "17.43", "0.0", "0.0", "1.33", "16.10", "83.90",
    );
    check_ip(
        &model,
        0,
        1,
        "17.43",
        "0.013315561644",
        "1.3315561644",
        "1.33",
        "16.10",
        "83.90",
    );
    check_ip(
        &model,
        1,
        0,
        "17.43",
        "0.012456493151",
        "1.04509977537",
        "1.05",
        "16.38",
        "67.52",
    );
    check_ip(
        &model,
        2,
        0,
        "17.43",
        "0.013315561644",
        "0.899066722202",
        "0.90",
        "16.53",
        "50.99",
    );
    check_ip(
        &model,
        3,
        0,
        "17.43",
        "0.012886027397",
        "0.657058536972",
        "0.66",
        "16.77",
        "34.22",
    );
    check_ip(
        &model,
        4,
        0,
        "17.43",
        "0.013315561644",
        "0.455658519458",
        "0.46",
        "16.97",
        "17.25",
    );
    check_ip(
        &model,
        5,
        0,
        "17.47",
        "0.012886027397",
        "0.222283972598",
        "0.22",
        "17.25",
        "0.00",
    );

    // repay EMI on first period due date
    model.pay_principal(jan1, feb1, feb1, &m("16.1"));
    model.pay_interest(jan1, feb1, feb1, &m("1.33"));
    // repay EMI on first period due date - next installment for 2nd period
    model.pay_principal(feb1, mar1, feb1, &m("17.43"));
    // repay last/penultimate installments on first period due date -- ie MIR
    model.pay_principal(jun1, jul1, feb1, &m("17.43"));
    model.pay_principal(may1, jun1, feb1, &m("2.57"));

    let transaction_date = jul10;
    let (p_before, i_before) = model.get_outstanding_amounts_till_date(transaction_date);
    model.re_age_equal_amortization(transaction_date, jul15, 6, 1, false);
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

// ----- helpers reused: bd, d, m closure, check_rp, check_ip -----

fn three_monthly(start_y: i32, start_m: Month) -> Vec<(Date, Date)> {
    // expectedRepaymentsMonthly(disbursementDate, 3, 1)
    let plus = |n: i64| {
        // disbursementDate.plusMonths(n) for day-1 dates
        let mut y = start_y;
        let mut mo = start_m as u8 as i64 - 1 + n;
        y += (mo / 12) as i32;
        mo %= 12;
        d(y, Month::try_from((mo + 1) as u8).unwrap(), 1)
    };
    vec![(plus(0), plus(1)), (plus(1), plus(2)), (plus(2), plus(3))]
}

// ============ DECLINING (Java class DecliningBalanceSameAsRepaymentPeriod) ============
// @BeforeEach (lines 3977-3981): DECLINING_BALANCE, SAME_AS_REPAYMENT_PERIOD, ACTUAL/ACTUAL.

/// Java test_3_of_1_month_period_multi_disbursement__exact_days (lines 3984-4020).
#[test]
fn same_as_repayment_declining_multi_disbursement_exact_days() {
    let periods = three_monthly(2024, Month::June);
    let jun1 = d(2024, Month::June, 1);
    let jul15 = d(2024, Month::July, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ true,
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jun1, &m("500.0")); // line 4004
    check_rp(&model, 0, "170.01", "5.00", "165.01", "334.99"); // 4005
    check_rp(&model, 1, "170.01", "3.35", "166.66", "168.33"); // 4006
    check_rp(&model, 2, "170.01", "1.68", "168.33", "0.00"); // 4007

    model.add_disbursement(jun1, &m("500.0")); // 4009
    check_rp(&model, 0, "340.02", "10.00", "330.02", "669.98"); // 4011
    check_rp(&model, 1, "340.02", "6.70", "333.32", "336.66"); // 4012
    check_rp(&model, 2, "340.03", "3.37", "336.66", "0.00"); // 4013

    model.add_disbursement(jul15, &m("250.0")); // 4016
    check_rp(&model, 0, "340.02", "10.00", "330.02", "669.98"); // 4017
    check_rp(&model, 1, "466.33", "8.07", "458.26", "461.72"); // 4018
    check_rp(&model, 2, "466.34", "4.62", "461.72", "0.00"); // 4019
}

/// Java test_3_of_1_month_period_multi_disbursement__exact_days__on_period_due_date (lines 4022-4057).
#[test]
fn same_as_repayment_declining_multi_disbursement_exact_days_on_period_due_date() {
    let periods = three_monthly(2024, Month::June);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        true,
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jun1, &m("500.0")); // 4042
    check_rp(&model, 0, "170.01", "5.00", "165.01", "334.99"); // 4043
    check_rp(&model, 1, "170.01", "3.35", "166.66", "168.33"); // 4044
    check_rp(&model, 2, "170.01", "1.68", "168.33", "0.00"); // 4045

    model.add_disbursement(jun1, &m("500.0")); // 4047
    check_rp(&model, 0, "340.02", "10.00", "330.02", "669.98"); // 4048
    check_rp(&model, 1, "340.02", "6.70", "333.32", "336.66"); // 4049
    check_rp(&model, 2, "340.03", "3.37", "336.66", "0.00"); // 4050

    model.add_disbursement(jul1, &m("250.0")); // 4053
    check_rp(&model, 0, "340.02", "10.00", "330.02", "919.98"); // 4054
    check_rp(&model, 1, "466.90", "9.20", "457.70", "462.28"); // 4055
    check_rp(&model, 2, "466.90", "4.62", "462.28", "0.00"); // 4056
}

/// Java test_3_of_1_month_period_multi_disbursement__NOT_exact_days__different_days (lines 4059-4116).
#[test]
fn same_as_repayment_declining_multi_disbursement_not_exact_days_different_days() {
    let periods = three_monthly(2024, Month::June);
    let jun1 = d(2024, Month::June, 1);
    let jul1 = d(2024, Month::July, 1);
    let jul15 = d(2024, Month::July, 15);
    let jul23 = d(2024, Month::July, 23);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ false,
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jun1, &m("500.0")); // 4079
    check_rp(&model, 0, "170.01", "5.00", "165.01", "334.99"); // 4080
    check_rp(&model, 1, "170.01", "3.35", "166.66", "168.33"); // 4081
    check_rp(&model, 2, "170.01", "1.68", "168.33", "0.00"); // 4082

    model.add_disbursement(jun1, &m("500.0")); // 4084
    check_rp(&model, 0, "340.02", "10.00", "330.02", "669.98"); // 4085
    check_rp(&model, 1, "340.02", "6.70", "333.32", "336.66"); // 4086
    check_rp(&model, 2, "340.03", "3.37", "336.66", "0.00"); // 4087

    model.add_disbursement(jul1, &m("50.0")); // 4092
    check_rp(&model, 0, "340.02", "10.00", "330.02", "719.98"); // 4093
    check_rp(&model, 1, "365.40", "7.20", "358.20", "361.78"); // 4094
    check_rp(&model, 2, "365.40", "3.62", "361.78", "0.00"); // 4095

    model.add_disbursement(jul15, &m("50.0")); // 4097
    check_rp(&model, 0, "340.02", "10.00", "330.02", "769.98"); // 4098
    check_rp(&model, 1, "390.77", "7.70", "383.07", "386.91"); // 4099
    check_rp(&model, 2, "390.78", "3.87", "386.91", "0.00"); // 4100

    model.add_disbursement(jul15, &m("50.0")); // 4102
    check_rp(&model, 0, "340.02", "10.00", "330.02", "819.98"); // 4103
    check_rp(&model, 1, "416.15", "8.20", "407.95", "412.03"); // 4104
    check_rp(&model, 2, "416.15", "4.12", "412.03", "0.00"); // 4105

    model.add_disbursement(jul23, &m("50.0")); // 4107
    check_rp(&model, 0, "340.02", "10.00", "330.02", "869.98"); // 4108
    check_rp(&model, 1, "441.53", "8.70", "432.83", "437.15"); // 4109
    check_rp(&model, 2, "441.52", "4.37", "437.15", "0.00"); // 4110

    model.add_disbursement(jul23, &m("50.0")); // 4112
    check_rp(&model, 0, "340.02", "10.00", "330.02", "919.98"); // 4113
    check_rp(&model, 1, "466.90", "9.20", "457.70", "462.28"); // 4114
    check_rp(&model, 2, "466.90", "4.62", "462.28", "0.00"); // 4115
}

/// Java test_3_of_1_month_period_multi_disbursement__NOT_exact_days (lines 4118-4158).
#[test]
fn same_as_repayment_declining_multi_disbursement_not_exact_days() {
    let periods = three_monthly(2024, Month::June);
    let jun1 = d(2024, Month::June, 1);
    let jul15 = d(2024, Month::July, 15);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ false,
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jun1, &m("500.0")); // 4138
    check_rp(&model, 0, "170.01", "5.00", "165.01", "334.99"); // 4139
    check_rp(&model, 1, "170.01", "3.35", "166.66", "168.33"); // 4140
    check_rp(&model, 2, "170.01", "1.68", "168.33", "0.00"); // 4141

    model.add_disbursement(jun1, &m("500.0")); // 4143
    check_rp(&model, 0, "340.02", "10.00", "330.02", "669.98"); // 4144
    check_rp(&model, 1, "340.02", "6.70", "333.32", "336.66"); // 4145
    check_rp(&model, 2, "340.03", "3.37", "336.66", "0.00"); // 4146

    model.add_disbursement(jul15, &m("130.0")); // 4149
    check_rp(&model, 0, "340.02", "10.00", "330.02", "799.98"); // 4150
    check_rp(&model, 1, "406.00", "8.00", "398.00", "401.98"); // 4151
    check_rp(&model, 2, "406.00", "4.02", "401.98", "0.00"); // 4152

    model.add_disbursement(jul15, &m("120.0")); // 4154
    check_rp(&model, 0, "340.02", "10.00", "330.02", "919.98"); // 4155
    check_rp(&model, 1, "466.90", "9.20", "457.70", "462.28"); // 4156
    check_rp(&model, 2, "466.90", "4.62", "462.28", "0.00"); // 4157
}

// ============ FLAT (Java class InterestTypeFlatAndCalculationPeriodSameAsRepaymentPeriod) ============
// @BeforeEach (lines 3605-3616): FLAT, SAME_AS_REPAYMENT_PERIOD, ACTUAL/ACTUAL, allowPartial=true (overridden per test).

/// Java test_sameAsRepayment_month_repay_every_1_periods_3_second_disbursement_on_repayment_due_date_allow_partial
/// (lines 3710-3742).
#[test]
fn same_as_repayment_flat_periods_3_second_disbursement_on_due_date_allow_partial() {
    let periods = three_monthly(2024, Month::January);
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1); // disbursementDate.plusMonths(1)
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ true,
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("1000.0")); // 3731
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67"); // 3733
    check_rp(&model, 1, "343.33", "10.00", "333.33", "333.34"); // 3734
    check_rp(&model, 2, "343.34", "10.00", "333.34", "0.00"); // 3735

    model.add_disbursement(feb1, &m("250.0")); // 3737
    check_rp(&model, 0, "343.33", "10.00", "333.33", "916.67"); // 3739
    check_rp(&model, 1, "470.84", "12.50", "458.34", "458.33"); // 3740
    check_rp(&model, 2, "470.83", "12.50", "458.33", "0.00"); // 3741
}

/// Java ..._second_disbursement_on_repayment_due_date_disallow_partial (lines 3744-3776).
#[test]
fn same_as_repayment_flat_periods_3_second_disbursement_on_due_date_disallow_partial() {
    let periods = three_monthly(2024, Month::January);
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ false,
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("1000.0")); // 3765
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67"); // 3767
    check_rp(&model, 1, "343.33", "10.00", "333.33", "333.34"); // 3768
    check_rp(&model, 2, "343.34", "10.00", "333.34", "0.00"); // 3769

    model.add_disbursement(feb1, &m("250.0")); // 3771
    check_rp(&model, 0, "343.33", "10.00", "333.33", "916.67"); // 3773
    check_rp(&model, 1, "470.84", "12.50", "458.34", "458.33"); // 3774
    check_rp(&model, 2, "470.83", "12.50", "458.33", "0.00"); // 3775
}

/// Java test_sameAsRepayment_month_repay_every_1_periods_3 (lines 3778-3815). allowPartial=true (from @BeforeEach).
#[test]
fn same_as_repayment_flat_periods_3_mid_period_disbursements() {
    let periods = three_monthly(2024, Month::January);
    let jan1 = d(2024, Month::January, 1);
    let jan5 = d(2024, Month::January, 5); // disbursementDate.plusDays(4)
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ true,
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("1000.0")); // 3797
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67"); // 3799
    check_rp(&model, 1, "343.33", "10.00", "333.33", "333.34"); // 3800
    check_rp(&model, 2, "343.34", "10.00", "333.34", "0.00"); // 3801

    model.add_disbursement(jan5, &m("250.0")); // 3803
    check_rp(&model, 0, "429.06", "12.18", "416.88", "833.12"); // 3805
    check_rp(&model, 1, "429.06", "12.50", "416.56", "416.56"); // 3806
    check_rp(&model, 2, "429.06", "12.50", "416.56", "0.00"); // 3807

    model.add_disbursement(jan5, &m("250.0")); // 3809
    check_rp(&model, 0, "514.78", "14.35", "500.43", "999.57"); // 3811
    check_rp(&model, 1, "514.78", "15.00", "499.78", "499.79"); // 3812
    check_rp(&model, 2, "514.79", "15.00", "499.79", "0.00"); // 3813
}

/// Java test_sameAsRepayment_month_repay_every_1_periods_3__not_calculate_exact_days (lines 3933-3969).
#[test]
fn same_as_repayment_flat_periods_3_not_calculate_exact_days() {
    let periods = three_monthly(2024, Month::January);
    let jan1 = d(2024, Month::January, 1);
    let jan5 = d(2024, Month::January, 5); // plusDays(4)
    let jan18 = d(2024, Month::January, 18); // plusDays(17)
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ false,
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("1000.0")); // 3953
    check_rp(&model, 0, "343.33", "10.00", "333.33", "666.67"); // 3955
    check_rp(&model, 1, "343.33", "10.00", "333.33", "333.34"); // 3956
    check_rp(&model, 2, "343.34", "10.00", "333.34", "0.00"); // 3957

    model.add_disbursement(jan5, &m("250.0")); // 3959
    check_rp(&model, 0, "429.17", "12.50", "416.67", "833.33"); // 3960
    check_rp(&model, 1, "429.17", "12.50", "416.67", "416.66"); // 3961
    check_rp(&model, 2, "429.16", "12.50", "416.66", "0.00"); // 3962

    model.add_disbursement(jan18, &m("250.0")); // 3964
    check_rp(&model, 0, "515.00", "15.00", "500.00", "1000.00"); // 3965
    check_rp(&model, 1, "515.00", "15.00", "500.00", "500.00"); // 3966
    check_rp(&model, 2, "515.00", "15.00", "500.00", "0.00"); // 3967
}

/// Java test_sameAsRepayment_month_repay_every_1_periods_24, second-disbursement block (lines 3859-3931).
/// Uses 24 monthly periods; only the post-3rd-disbursement (Apr-5 = plusMonths(3).plusDays(4)) assertions shown for the
/// second disbursement focus. allowPartial=true (from @BeforeEach).
#[test]
fn same_as_repayment_flat_periods_24_second_disbursement() {
    // expectedRepaymentsMonthly(2024-01-01, 24, 1)
    let periods: Vec<(Date, Date)> = (0..24)
        .map(|i| {
            let from_total = i;
            let due_total = i + 1;
            let mk = |t: i64| {
                let y = 2024 + (t / 12) as i32;
                let mo = (t % 12) as u8 + 1;
                d(y, Month::try_from(mo).unwrap(), 1)
            };
            (mk(from_total), mk(due_total))
        })
        .collect();
    let jan1 = d(2024, Month::January, 1);
    let apr5 = d(2024, Month::April, 5); // plusMonths(3).plusDays(4)
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty_same_as_repayment(
        &periods,
        &bd("12.0"),
        1,
        12,
        /*allow_partial*/ true,
        InterestMethod::Flat,
        2,
        RoundingMode::HalfEven,
    );

    model.add_disbursement(jan1, &m("7500.0")); // 3877
    model.add_disbursement(jan1, &m("2500.0")); // 3878
    check_rp(&model, 0, "516.67", "100.00", "416.67", "9583.33"); // 3880
    check_rp(&model, 23, "516.59", "100.00", "416.59", "0.00"); // 3903 (tail)

    model.add_disbursement(apr5, &m("1000.0")); // 3905
    check_rp(&model, 0, "516.67", "100.00", "416.67", "9583.33"); // 3907
    check_rp(&model, 1, "516.67", "100.00", "416.67", "9166.66"); // 3908
    check_rp(&model, 2, "516.67", "100.00", "416.67", "8749.99"); // 3909
    check_rp(&model, 3, "574.22", "108.67", "465.55", "9284.44"); // 3910
    check_rp(&model, 4, "574.22", "110.00", "464.22", "8820.22"); // 3911
    check_rp(&model, 23, "574.26", "110.00", "464.26", "0.00"); // 3930 (tail)
}

/// `test_interestGraceForProgressiveSchedule` (ProgressiveEMICalculatorTest.java:5317-5345): 100 @ 7% DAYS_360/30, six
/// monthly periods, graceOnInterestPayment = 2 → periods 0/1 accrue no due interest (deferred forward), period 2's
/// interest carries the deferral (1.44). EMI stays 17.01 (16.98 on the last). Exercises apply_interest_moratorium +
/// the grace-aware annuity (rate_factor_plus_1_for_emi).
#[test]
fn interest_grace_for_progressive_schedule() {
    let m = |s: &str| fc_money::Money::of(2, RoundingMode::HalfEven, &bd(s));
    let mut model = new_empty(
        &six_monthly(),
        &bd("7.0"),
        DayCount::Days360Days30 { repay_every: 1 },
        InterestMethod::DecliningBalance,
        2,
        RoundingMode::HalfEven,
    );
    model.grace_on_interest = 2;
    model.interest_pause_for_emi = true;
    model.add_disbursement(d(2024, Month::January, 1), &m("100.0"));

    check_rp(&model, 0, "17.01", "0.00", "17.01", "82.99");
    check_rp(&model, 1, "17.01", "0.00", "17.01", "65.98");
    check_rp(&model, 2, "17.01", "1.44", "15.57", "50.41");
    check_rp(&model, 3, "17.01", "0.29", "16.72", "33.69");
    check_rp(&model, 4, "17.01", "0.20", "16.81", "16.88");
    check_rp(&model, 5, "16.98", "0.10", "16.88", "0.00");
}
