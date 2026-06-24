//! Conformance for the equal-amortization split (`EqualAmortizationValues` /
//! `ProgressiveEMICalculator.calculateEqualAmortizationValues`). Expectations are computed from the exact (pure)
//! algorithm — `value = round2(total / n)`, `adjustment = total − value × n` — which is a direct line-by-line port of
//! the original. Currency USD/2, HALF_EVEN (matching `MoneyHelper.getMathContext()` in the original tests).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::{
    EqualAmortizationValues, calculate_adjusted_equal_amortization_values,
    calculate_equal_amortization_values,
};
use fc_money::Money;

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn money(v: &str) -> Money {
    Money::of(
        SCALE,
        MODE,
        &BigDecimal::parse_bytes(v.as_bytes(), 10).unwrap(),
    )
}

fn eav(total: &str, n: i64, value: &str, adjustment: &str) -> EqualAmortizationValues {
    EqualAmortizationValues {
        total_outstanding: money(total),
        number_of_installments: n,
        value: money(value),
        adjustment: money(adjustment),
    }
}

#[test]
fn even_split_no_remainder() {
    let r = calculate_equal_amortization_values(&money("1000"), 4);
    assert_eq!(r.value.to_plain_string(), "250.00");
    assert_eq!(r.adjustment.to_plain_string(), "0.00");
    assert_eq!(r.adjusted_value().to_plain_string(), "250.00");
    for i in 0..4 {
        assert_eq!(
            r.calculate_value(i).to_plain_string(),
            "250.00",
            "installment {i}"
        );
    }
}

#[test]
fn split_with_rounding_remainder() {
    // 1000 / 3 = 333.33 (HALF_EVEN @2); adjustment = 1000.00 − 333.33×3 = 0.01; last installment = 333.34.
    let r = calculate_equal_amortization_values(&money("1000"), 3);
    assert_eq!(r.value.to_plain_string(), "333.33");
    assert_eq!(r.adjustment.to_plain_string(), "0.01");
    assert_eq!(r.adjusted_value().to_plain_string(), "333.34");
    assert_eq!(r.calculate_value(0).to_plain_string(), "333.33");
    assert_eq!(r.calculate_value(1).to_plain_string(), "333.33");
    assert_eq!(r.calculate_value(2).to_plain_string(), "333.34"); // last gets the adjustment
}

#[test]
fn small_amount_split() {
    let r = calculate_equal_amortization_values(&money("10"), 3);
    assert_eq!(r.value.to_plain_string(), "3.33");
    assert_eq!(r.adjustment.to_plain_string(), "0.01");
    assert_eq!(r.adjusted_value().to_plain_string(), "3.34");
}

#[test]
fn zero_total_yields_zero_split() {
    let r = calculate_equal_amortization_values(&money("0"), 3);
    assert_eq!(r.value.to_plain_string(), "0.00");
    assert_eq!(r.adjustment.to_plain_string(), "0.00");
    assert_eq!(r.calculate_value(2).to_plain_string(), "0.00");
}

#[test]
fn add_combines_matching_counts() {
    let a = eav("600", 3, "200", "0");
    let b = eav("300", 3, "100", "0");
    let c = a.try_add(&b).expect("matching counts");
    assert_eq!(c.total_outstanding.to_plain_string(), "900.00");
    assert_eq!(c.value.to_plain_string(), "300.00");
    assert_eq!(c.adjustment.to_plain_string(), "0.00");
}

#[test]
fn add_rejects_mismatched_counts() {
    let a = eav("600", 3, "200", "0");
    let b = eav("300", 4, "75", "0");
    assert!(a.try_add(&b).is_err());
}

#[test]
fn adjusted_split_reduces_value_by_other_components() {
    // total 1000 / 3 = 333.33; value = 333.33 − 100 = 233.33; adjust = 900 − 233.33×3 = 200.01.
    let r = calculate_adjusted_equal_amortization_values(
        &money("900"),
        &money("1000"),
        &money("100"),
        3,
    );
    assert_eq!(r.value.to_plain_string(), "233.33");
    assert_eq!(r.adjustment.to_plain_string(), "200.01");
    assert_eq!(r.total_outstanding.to_plain_string(), "900.00");
}

#[test]
fn adjusted_split_even() {
    // total 600 / 2 = 300; value = 300 − 50 = 250; adjust = 500 − 250×2 = 0.
    let r =
        calculate_adjusted_equal_amortization_values(&money("500"), &money("600"), &money("50"), 2);
    assert_eq!(r.value.to_plain_string(), "250.00");
    assert_eq!(r.adjustment.to_plain_string(), "0.00");
}

/// The degenerate negative-adjusted-value branch of `calculateValue`: installments are paid `value` until the total is
/// consumed, then zero.
#[test]
fn calculate_value_negative_adjusted_branch() {
    // adjusted_value = 100 + (−200) = −100 (< 0). total 150, value 100.
    let r = eav("150", 3, "100", "-200");
    assert!(r.adjusted_value().is_less_than_zero());
    assert_eq!(r.calculate_value(0).to_plain_string(), "100.00"); // 150 − 100×1 = 50 ≥ 0 → value
    assert_eq!(r.calculate_value(2).to_plain_string(), "0.00"); // 150 − 100×3 = −150 < 0 → zero
}

/// `EqualAmortizationValue.test_AmortizationTotalIsLessThanInstallmentNumber` (test 5103): 0.04 over 6 installments —
/// value 0.01, adjusted value 0.01+(-0.02)=-0.01 (< 0) → degenerate branch pays 0.01 until 0.04 is consumed, then 0.00.
#[test]
fn amortization_total_less_than_installment_number() {
    let r = calculate_equal_amortization_values(&money("0.04"), 6);
    let expected = ["0.01", "0.01", "0.01", "0.01", "0.00", "0.00"];
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(
            r.calculate_value(i as i64).to_plain_string(),
            *e,
            "installment {i}"
        );
    }
}

/// `EqualAmortizationValue.test_AmortizationIsJustBiggerThanInstallmentNumber` (test 5116): 0.07 over 6 — value 0.01,
/// adjusted value 0.01+0.01=0.02 (≥ 0) → first five 0.01, last installment 0.02.
#[test]
fn amortization_just_bigger_than_installment_number() {
    let r = calculate_equal_amortization_values(&money("0.07"), 6);
    let expected = ["0.01", "0.01", "0.01", "0.01", "0.01", "0.02"];
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(
            r.calculate_value(i as i64).to_plain_string(),
            *e,
            "installment {i}"
        );
    }
}

/// `EqualAmortizationValue.test_AmortizationNonEdgeCase` (test 5129): 0.59 over 6 — value 0.10, adjusted value
/// 0.10+(-0.01)=0.09 (≥ 0) → first five 0.10, last installment 0.09.
#[test]
fn amortization_non_edge_case() {
    let r = calculate_equal_amortization_values(&money("0.59"), 6);
    let expected = ["0.10", "0.10", "0.10", "0.10", "0.10", "0.09"];
    for (i, e) in expected.iter().enumerate() {
        assert_eq!(
            r.calculate_value(i as i64).to_plain_string(),
            *e,
            "installment {i}"
        );
    }
}
