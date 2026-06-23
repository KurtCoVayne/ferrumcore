//! Conformance for the declining-balance per-period interest formula
//! (`InterestPeriod.getCalculatedDueInterest`). Expectations computed from the exact `MathContext(12, HALF_EVEN)`
//! arithmetic of the original formula. Rate factor 0.00583333333333 corresponds to 7% nominal, DAYS_360/DAYS_30.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::Mc;
use fc_loan_emi::interest_period::calculated_due_interest_declining;
use fc_money::decimal::to_plain_string;
use std::str::FromStr;

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}

#[test]
fn full_period_interest() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let r = calculated_due_interest_declining(
        &mc,
        &bd("100"),
        &bd("0.00583333333333"),
        30,
        30,
        &bd("0"),
    );
    assert_eq!(to_plain_string(&r), "0.583333333332");
}

#[test]
fn partial_period_interest() {
    // half the period (15 of 30 days).
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let r = calculated_due_interest_declining(
        &mc,
        &bd("100"),
        &bd("0.00583333333333"),
        15,
        30,
        &bd("0"),
    );
    assert_eq!(to_plain_string(&r), "0.291666666666");
}

#[test]
fn interest_plus_credited() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let r = calculated_due_interest_declining(
        &mc,
        &bd("100"),
        &bd("0.00583333333333"),
        30,
        30,
        &bd("0.10"),
    );
    assert_eq!(to_plain_string(&r), "0.683333333332");
}

#[test]
fn zero_length_till_due_yields_credited_only() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let r = calculated_due_interest_declining(
        &mc,
        &bd("100"),
        &bd("0.00583333333333"),
        30,
        0,
        &bd("0.25"),
    );
    // Faithful Mc does not pad: credited-only interest renders as "0.25" (matching Java's no-pad round).
    assert_eq!(to_plain_string(&r), "0.25");
}

#[test]
fn negative_sum_floored_to_zero() {
    // credited interest negative enough to make the sum negative → floored at zero.
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let r = calculated_due_interest_declining(
        &mc,
        &bd("100"),
        &bd("0.00583333333333"),
        30,
        30,
        &bd("-1"),
    );
    assert_eq!(to_plain_string(&r), "0");
}
