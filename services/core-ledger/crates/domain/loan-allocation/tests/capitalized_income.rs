//! Conformance: `CapitalizedIncomeAmortizationUtil.calculateTotalAmortizationTillDate` (EQUAL_AMORTIZATION) against
//! goldens captured from a JVM `BigDecimal` replay of the exact Java arithmetic (jshell, JDK 21.0.5 — the util is a
//! pure function of BigDecimal math at `MathContext(12, HALF_EVEN)` plus day arithmetic, so the JVM replay IS the
//! ground truth for the rounding behavior).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::capitalized_income::{
    CapitalizedIncomeAdjustment, calculate_total_amortization_till_date,
};
use fc_money::Money;
use std::str::FromStr;
use time::{Date, Month};

fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &BigDecimal::from_str(s).unwrap())
}

fn d(y: i32, mo: Month, day: u8) -> Date {
    Date::from_calendar_date(y, mo, day).unwrap()
}

/// The four JVM-captured goldens: A1 an adjustment mid-life then a tail proration (53.28), A2 the pure linear case
/// (120 × 91/182 = 60.00 exactly), A3 an over-adjustment carried as a negative correction (5.00), A4 a balance
/// starting on the maturity date recognizing in full (120.00).
#[test]
fn equal_amortization_matches_jvm_goldens() {
    let jan1 = d(2024, Month::January, 1);
    let jul1 = d(2024, Month::July, 1);
    let apr1 = d(2024, Month::April, 1);

    // A1: 120 from Jan 1, adjustment 20 on Feb 15, till Apr 1 → 53.28.
    let a1 = calculate_total_amortization_till_date(
        &m("120"),
        jan1,
        &[CapitalizedIncomeAdjustment {
            date: d(2024, Month::February, 15),
            amount: m("20"),
        }],
        jul1,
        apr1,
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(a1.to_plain_string(), "53.28");

    // A2: no adjustments → pure linear 120 × 91/182 = 60.00.
    let a2 = calculate_total_amortization_till_date(
        &m("120"),
        jan1,
        &[],
        jul1,
        apr1,
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(a2.to_plain_string(), "60.00");

    // A3: over-adjustment 115 on Jun 15 (unrecognized goes negative → correction), till Jun 20 → 5.00.
    let a3 = calculate_total_amortization_till_date(
        &m("120"),
        jan1,
        &[CapitalizedIncomeAdjustment {
            date: d(2024, Month::June, 15),
            amount: m("115"),
        }],
        jul1,
        d(2024, Month::June, 20),
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(a3.to_plain_string(), "5.00");

    // A4: balance dated on the maturity date recognizes in full.
    let a4 = calculate_total_amortization_till_date(
        &m("120"),
        jul1,
        &[],
        jul1,
        jul1,
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(a4.to_plain_string(), "120.00");
}

/// The buy-down-fee amortization alias resolves to the same verified function (Java duplicates the arithmetic
/// character-for-character across `CapitalizedIncomeAmortizationUtil` / `BuyDownFeeAmortizationUtil`).
#[test]
fn buy_down_fee_amortization_is_the_same_function() {
    use fc_loan_allocation::capitalized_income::calculate_buy_down_fee_amortization_till_date;
    let jan1 = d(2024, Month::January, 1);
    let jul1 = d(2024, Month::July, 1);
    let out = calculate_buy_down_fee_amortization_till_date(
        &m("120"),
        jan1,
        &[],
        jul1,
        d(2024, Month::April, 1),
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(out.to_plain_string(), "60.00");
}
