//! Conformance for the declining-balance schedule model, replaying the VERIFIED engine test
//! `ProgressiveEMICalculatorTest.test_emi_calculator_performance`: 100 disbursed over 12 monthly periods at 7% nominal,
//! DAYS_360 / DAYS_30. Asserts: 12 periods, loan term 366 days, EMI 8.65 on every non-final period, every period has
//! positive due principal and due interest, and the final period's outstanding balance is zero.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::{DayCount, generate_declining_balance};
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
        DayCount {
            days_in_year: 360,
            days_in_month: 30,
            days_in_period: 30,
        },
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

fn day_count() -> DayCount {
    DayCount {
        days_in_year: 360,
        days_in_month: 30,
        days_in_period: 30,
    }
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
