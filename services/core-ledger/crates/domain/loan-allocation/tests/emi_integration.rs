//! Integration: the real `fc-loan-emi` engine wired behind the Advanced processor's [`EmiCalculator`] seam via
//! [`ProgressiveEmiCalculator`]. Confirms the seam operations (driven as `&mut dyn EmiCalculator`, exactly as the
//! Advanced processor calls them) actually mutate the underlying progressive interest schedule.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::ProgressiveEmiCalculator;
use fc_loan_allocation::advanced::{EmiCalculator, ScheduleModel};
use fc_loan_emi::recalc::new_declining_balance;
use fc_money::Money;
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn d(y: i32, m: Month, day: u8) -> Date {
    Date::from_calendar_date(y, m, day).unwrap()
}
fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &bd(s))
}

/// A 100 @ 7% DAYS_360/30 declining loan (6 monthly periods, EMI 17.01) driven through the seam: paying period 0's
/// principal + interest via `&mut dyn EmiCalculator` marks it fully paid in the real engine, and the outstanding
/// cascades to the next period — proving the production calculator is wired correctly.
#[test]
fn real_engine_behind_emi_seam_records_payment() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ];
    let model = new_declining_balance(
        &periods,
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    // Fresh schedule: period 0 due is 16.43 principal + 0.58 interest, EMI 17.01, nothing paid yet.
    assert_eq!(model.period_due_principal(0).to_plain_string(), "16.43");
    assert_eq!(model.period_due_interest(0).to_plain_string(), "0.58");
    assert!(!model.period_fully_paid(0));

    let mut calc = ProgressiveEmiCalculator::new(model);
    let schedule = ScheduleModel {
        maturity_date: d(2024, Month::July, 1),
    };
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);

    // Drive the payment through the seam exactly as the Advanced processor does (as a trait object).
    let seam: &mut dyn EmiCalculator = &mut calc;
    seam.pay_principal(&schedule, jan1, feb1, feb1, &m("16.43"));
    seam.pay_interest(&schedule, jan1, feb1, feb1, &m("0.58"));

    // The real engine recorded the payment on period 0.
    assert!(
        calc.model().period_fully_paid(0),
        "period 0 fully paid after seam payment"
    );
    // ... and the remaining schedule is intact (period 1 still carries its EMI-derived due principal).
    assert!(
        !calc.model().period_fully_paid(1),
        "period 1 still outstanding"
    );
    assert_eq!(
        calc.model().period_due_interest(1).to_plain_string(),
        "0.49"
    );
}

/// A second disbursement driven through the seam re-amortizes the real engine (the merge path), proving
/// `add_disbursement` is wired, not a no-op.
#[test]
fn real_engine_behind_emi_seam_records_disbursement() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ];
    let model = new_declining_balance(
        &periods,
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let emi_before = model.period_emi(0);

    let mut calc = ProgressiveEmiCalculator::new(model);
    let schedule = ScheduleModel {
        maturity_date: d(2024, Month::July, 1),
    };
    let seam: &mut dyn EmiCalculator = &mut calc;
    // A 2nd disbursement on a period due date re-amortizes the schedule upward.
    seam.add_disbursement(&schedule, d(2024, Month::March, 1), &m("100.0"));

    assert!(
        calc.model().period_emi(2).is_greater_than(&emi_before),
        "2nd disbursement raised the EMI in the real engine"
    );
}
