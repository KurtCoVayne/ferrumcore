//! Conformance for the declining-balance schedule model, replaying the VERIFIED engine test
//! `ProgressiveEMICalculatorTest.test_emi_calculator_performance`: 100 disbursed over 12 monthly periods at 7% nominal,
//! DAYS_360 / DAYS_30. Asserts: 12 periods, loan term 366 days, EMI 8.65 on every non-final period, every period has
//! positive due principal and due interest, and the final period's outstanding balance is zero.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::{DayCountKind, generate_declining_balance};
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn first_of(year: i32, month: Month) -> Date {
    Date::from_calendar_date(year, month, 1).unwrap()
}
fn d(year: i32, month: Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).unwrap()
}

/// `checkPeriod(model, idx, emi, dueInterest, duePrincipal, outstanding)` from the original test.
fn check_period(
    model: &fc_loan_emi::Model,
    idx: usize,
    emi: &str,
    due_interest: &str,
    due_principal: &str,
    outstanding: &str,
) {
    let p = &model.periods[idx];
    assert_eq!(p.emi.to_plain_string(), emi, "period {idx} emi");
    assert_eq!(
        p.due_interest.to_plain_string(),
        due_interest,
        "period {idx} interest"
    );
    assert_eq!(
        p.due_principal.to_plain_string(),
        due_principal,
        "period {idx} principal"
    );
    assert_eq!(
        p.outstanding_balance.to_plain_string(),
        outstanding,
        "period {idx} outstanding"
    );
}

/// `test_360_30_repayment_schedule_disbursement_month_end`: 2450 @ 9.99% nominal, DAYS_360/DAYS_30, six monthly
/// (month-end) periods, **HALF_UP**. Exact per-period golden from the original `checkPeriod` assertions.
#[test]
fn repayment_schedule_360_30_month_end() {
    let periods = [
        (d(2023, Month::October, 31), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 31)),
        (d(2023, Month::December, 31), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("2450"),
        &bd("9.99"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "420.31", "20.40", "399.91", "2050.09");
    check_period(&model, 1, "420.31", "17.07", "403.24", "1646.85");
    check_period(&model, 2, "420.31", "13.71", "406.60", "1240.25");
    check_period(&model, 3, "420.31", "10.33", "409.98", "830.27");
    check_period(&model, 4, "420.31", "6.91", "413.40", "416.87");
    check_period(&model, 5, "420.34", "3.47", "416.87", "0.00");
}

/// `test_360_30_repayment_schedule_disbursement_near_month_end`: 10000 @ 45% nominal, DAYS_360/DAYS_30, six
/// near-month-end periods, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_360_30_near_month_end() {
    let periods = [
        (d(2023, Month::October, 30), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 30)),
        (d(2023, Month::December, 30), d(2024, Month::January, 30)),
        (d(2024, Month::January, 30), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 30)),
        (d(2024, Month::March, 30), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("10000"),
        &bd("45.0"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "1892.12", "375.00", "1517.12", "8482.88");
    check_period(&model, 1, "1892.12", "318.11", "1574.01", "6908.87");
    check_period(&model, 2, "1892.12", "259.08", "1633.04", "5275.83");
    check_period(&model, 3, "1892.12", "197.84", "1694.28", "3581.55");
    check_period(&model, 4, "1892.12", "134.31", "1757.81", "1823.74");
    check_period(&model, 5, "1892.13", "68.39", "1823.74", "0.00");
}

/// `test_360_30_repayment_schedule_disbursement_repay_every_2_months`: 3333 @ 19.99% nominal, DAYS_360/DAYS_30,
/// repay every 2 months, six periods, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_360_30_repay_every_2_months() {
    let periods = [
        (d(2023, Month::October, 29), d(2023, Month::December, 29)),
        (d(2023, Month::December, 29), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::April, 29)),
        (d(2024, Month::April, 29), d(2024, Month::June, 29)),
        (d(2024, Month::June, 29), d(2024, Month::August, 29)),
        (d(2024, Month::August, 29), d(2024, Month::October, 29)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("3333"),
        &bd("19.99"),
        DayCountKind::Days360Days30 { repay_every: 2 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "622.04", "111.04", "511.00", "2822.00");
    check_period(&model, 1, "622.04", "94.02", "528.02", "2293.98");
    check_period(&model, 2, "622.04", "76.43", "545.61", "1748.37");
    check_period(&model, 3, "622.04", "58.25", "563.79", "1184.58");
    check_period(&model, 4, "622.04", "39.47", "582.57", "602.01");
    check_period(&model, 5, "622.07", "20.06", "602.01", "0.00");
}

/// `test_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month` (interest-period granularity, HALF_EVEN):
/// 100 @ 9.4822%, 360/30, six monthly periods from 2024-01-01. Asserts the per-interest-period rate factor and
/// `getCalculatedDueInterest`, including the zero-length disbursement stub (rp0 interest period 0).
#[test]
fn disbursed_100_360_30_interest_period_granularity() {
    let model = generate_declining_balance(
        &monthly_periods_2024_six(),
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
    );
    // rp0 has two interest periods: the disbursement stub (rate factor 0) then the real one.
    let rp0 = &model.periods[0];
    assert_eq!(
        rp0.interest_periods.len(),
        2,
        "rp0 has stub + real interest period"
    );
    assert_eq!(rp0.interest_periods[0].rate_factor.to_string(), "0");
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[0].calculated_due_interest),
        "0"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[1].rate_factor),
        "0.007901833333"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[1].calculated_due_interest),
        "0.790183333301"
    );
    // repayment-period level: emi 17.13, dueInterest 0.79, duePrincipal 16.34, outstanding 83.66.
    check_period(&model, 0, "17.13", "0.79", "16.34", "83.66");
    check_period(&model, 1, "17.13", "0.66", "16.47", "67.19");
    check_period(&model, 5, "17.13", "0.13", "17.00", "0.00");
    // later periods have a single interest period.
    assert_eq!(model.periods[1].interest_periods.len(), 1);
}

/// `test_disbursedAmt1000_NoInterest_repayEvery1Month`: 1000 at 0% over 4 monthly periods (ACTUAL/ACTUAL) → equal
/// principal of 250, zero interest, balance stepping to 0.
#[test]
fn disbursed_1000_no_interest() {
    let periods: Vec<(Date, Date)> = (0..4)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect();
    let model = generate_declining_balance(
        &periods,
        &bd("1000"),
        &bd("0"),
        DayCountKind::ActualActual,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "250.00", "0.00", "250.00", "750.00");
    check_period(&model, 1, "250.00", "0.00", "250.00", "500.00");
    check_period(&model, 2, "250.00", "0.00", "250.00", "250.00");
    check_period(&model, 3, "250.00", "0.00", "250.00", "0.00");
    assert_eq!(
        model.periods[0].interest_periods[0].rate_factor.to_string(),
        "0"
    );
}

/// `test_disbursedAmt100_dayInYears364_daysInMonthActual_repayEvery1Week`: 100 @ 9.4822%, DAYS_364 / ACTUAL, six
/// weekly periods, HALF_EVEN. Exact per-period golden incl. the interest-period rate factor for rp0.
#[test]
fn disbursed_100_364_weekly() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 8)),
        (d(2024, Month::January, 8), d(2024, Month::January, 15)),
        (d(2024, Month::January, 15), d(2024, Month::January, 22)),
        (d(2024, Month::January, 22), d(2024, Month::January, 29)),
        (d(2024, Month::January, 29), d(2024, Month::February, 5)),
        (d(2024, Month::February, 5), d(2024, Month::February, 12)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::ActualFixedYear { days_in_year: 364 },
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.periods[0].interest_periods[1].rate_factor),
        "0.001823500000"
    );
    check_period(&model, 0, "16.77", "0.18", "16.59", "83.41");
    check_period(&model, 1, "16.77", "0.15", "16.62", "66.79");
    check_period(&model, 2, "16.77", "0.12", "16.65", "50.14");
    check_period(&model, 3, "16.77", "0.09", "16.68", "33.46");
    check_period(&model, 4, "16.77", "0.06", "16.71", "16.75");
    check_period(&model, 5, "16.78", "0.03", "16.75", "0.00");
}

/// Six monthly periods from 2024-01-01.
fn monthly_periods_2024_six() -> Vec<(Date, Date)> {
    (0..6)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect()
}
fn month_of(n: u8) -> Month {
    Month::try_from(n).unwrap()
}

/// 12 monthly periods from 2024-01-01.
fn monthly_periods_2024() -> Vec<(Date, Date)> {
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
    (0..12)
        .map(|i| {
            (
                first_of(months[i].0, months[i].1),
                first_of(months[i + 1].0, months[i + 1].1),
            )
        })
        .collect()
}

fn day_count() -> DayCountKind {
    DayCountKind::Days360Days30 { repay_every: 1 }
}

/// `test_actual_actual_repayment_schedule_across_multiple_years` (generation block, before the payment): 5000 @ 9.99%
/// nominal, ACTUAL/ACTUAL, six monthly periods crossing the 2023→2024 boundary, HALF_UP. Exact per-period golden,
/// exercising the year-boundary partial-period split.
#[test]
fn repayment_schedule_actual_actual_across_years() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCountKind::ActualActual,
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "857.71", "41.05", "816.66", "4183.34");
    check_period(&model, 1, "857.71", "35.45", "822.26", "3361.08");
    check_period(&model, 2, "857.71", "28.44", "829.27", "2531.81");
    check_period(&model, 3, "857.71", "20.04", "837.67", "1694.14");
    check_period(&model, 4, "857.71", "14.33", "843.38", "850.76");
    check_period(&model, 5, "857.73", "6.97", "850.76", "0.00");
}

/// `test_actual_actual_repayment_schedule_disbursement_month_end` (generation block): 245000 @ 45% nominal,
/// ACTUAL/ACTUAL, six month-end periods crossing 2023→2024, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_actual_actual_month_end() {
    let periods = [
        (d(2023, Month::October, 31), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 31)),
        (d(2023, Month::December, 31), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("245000"),
        &bd("45.0"),
        DayCountKind::ActualActual,
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "46343.27", "9061.64", "37281.63", "207718.37");
    check_period(&model, 1, "46343.27", "7938.83", "38404.44", "169313.93");
    check_period(&model, 2, "46343.27", "6453.36", "39889.91", "129424.02");
    check_period(&model, 3, "46343.27", "4614.71", "41728.56", "87695.46");
    check_period(&model, 4, "46343.27", "3342.49", "43000.78", "44694.68");
    check_period(&model, 5, "46343.25", "1648.57", "44694.68", "0.00");
}

#[test]
fn due_amounts_first_period() {
    let model = generate_declining_balance(
        &monthly_periods_2024(),
        &bd("100"),
        &bd("7.0"),
        day_count(),
        2,
        RoundingMode::HalfEven,
    );
    let (from, due) = monthly_periods_2024()[0];
    let d = model.due_amounts(from, due).expect("first period");
    // EMI 8.65; interest on 100 @ 0.005833333333 → 0.58; principal = 8.65 − 0.58 = 8.07.
    assert_eq!(d.emi.to_plain_string(), "8.65");
    assert_eq!(d.due_interest.to_plain_string(), "0.58");
    assert_eq!(d.due_principal.to_plain_string(), "8.07");
}

#[test]
fn emi_calculator_performance_fresh_disbursement() {
    let day_count = day_count();
    let model = generate_declining_balance(
        &monthly_periods_2024(),
        &bd("100"),
        &bd("7.0"),
        day_count,
        2,
        RoundingMode::HalfEven,
    );

    assert_eq!(model.periods.len(), 12, "12 repayment periods");
    assert_eq!(
        model.loan_term_in_days(),
        366,
        "loan term in days (2024 leap year)"
    );

    let n = model.periods.len();
    for (i, p) in model.periods.iter().enumerate() {
        assert!(
            p.due_principal.is_greater_than_zero(),
            "period {} due principal > 0",
            i + 1
        );
        assert!(
            p.due_interest.is_greater_than_zero(),
            "period {} due interest > 0",
            i + 1
        );
        if i == n - 1 {
            assert_eq!(
                p.outstanding_balance.to_plain_string(),
                "0.00",
                "final period outstanding == 0"
            );
        } else {
            assert_eq!(p.emi.to_plain_string(), "8.65", "period {} EMI", i + 1);
            assert!(
                p.outstanding_balance.is_greater_than_zero(),
                "period {} outstanding > 0",
                i + 1
            );
        }
    }
}
