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

#[test]
fn emi_calculator_performance_fresh_disbursement() {
    let day_count = DayCount {
        days_in_year: 360,
        days_in_month: 30,
        days_in_period: 30,
    };
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
