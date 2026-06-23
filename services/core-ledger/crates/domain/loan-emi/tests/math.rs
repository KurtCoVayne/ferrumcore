//! Conformance for the faithful Java `MathContext` arithmetic ([`Mc`]). Goldens computed from a `prec=12,
//! ROUND_HALF_EVEN` decimal context (which mirrors `java.math.BigDecimal` + `MathContext` exactly), incl. `divide`'s
//! terminating-quotient preferred scale and the no-padding `round` rule.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::Mc;
use fc_money::decimal::to_plain_string;
use std::str::FromStr;

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}

#[test]
fn divide_terminating_keeps_preferred_scale() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let cases = [
        ("1", "4", "0.25"),
        ("10", "4", "2.5"),
        ("2", "1", "2"),
        ("1", "8", "0.125"),
        ("0", "7", "0"),
    ];
    for (a, b, want) in cases {
        assert_eq!(to_plain_string(&mc.div(&bd(a), &bd(b))), want, "{a}/{b}");
    }
}

#[test]
fn divide_nonterminating_rounds_to_precision() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let cases = [
        ("1", "3", "0.333333333333"),
        ("100", "3", "33.3333333333"),
        ("100", "12", "8.33333333333"),
        ("7", "12", "0.583333333333"),
    ];
    for (a, b, want) in cases {
        assert_eq!(to_plain_string(&mc.div(&bd(a), &bd(b))), want, "{a}/{b}");
    }
}

#[test]
fn round_does_not_pad_short_values() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    assert_eq!(to_plain_string(&mc.round(&bd("0.25"))), "0.25");
    assert_eq!(to_plain_string(&mc.round(&bd("2.5"))), "2.5");
    assert_eq!(to_plain_string(&mc.add(&bd("0.25"), &bd("0"))), "0.25");
}

#[test]
fn round_reduces_excess_significant_figures() {
    let mc = Mc::new(12, RoundingMode::HalfEven);
    // 13 significant figures → rounded to 12 (HALF_EVEN).
    assert_eq!(
        to_plain_string(&mc.round(&bd("1.008053375342"))),
        "1.00805337534"
    );
    assert_eq!(
        to_plain_string(&mc.mul(&bd("2.00753380274"), &bd("1.00805337534"))),
        "2.02370122596"
    );
}

#[test]
fn add_propagates_scale_then_rounds() {
    // 1 + 0E-11 = 1.00000000000 (12 sig figs, no reduction) — matches Java fnValue seed behaviour.
    let mc = Mc::new(12, RoundingMode::HalfEven);
    let zero_scaled = bd("0.00000000000"); // scale 11
    assert_eq!(
        to_plain_string(&mc.add(&BigDecimal::from(1), &zero_scaled)),
        "1.00000000000"
    );
}
