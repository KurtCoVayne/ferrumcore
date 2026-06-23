//! Characterization tests for the `early-repayment-strategy`
//! (`EarlyPaymentLoanRepaymentScheduleTransactionProcessor`).
//!
//! This strategy has NO original unit test, so the expectations below are NOT captured goldens. They are
//! CHARACTERIZATION values hand-traced by simulating the already-golden-verified [`Installment`] atoms through the
//! ported ordering:
//!   - on-time / late: interest → principal → penalty → fee;
//!   - in-advance: principal only (the entire remainder is offered to principal; interest/fee/penalty stay zero and
//!     due charges are ignored — the distinctive quirk of this processor).
//!
//! The scenario shapes mirror the Due* conformance suite: single-component, combined P/I/F/Pen, partial payments that
//! stop in each bucket, and in-advance vs on-time vs late. Currency USD/2, HALF_EVEN.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::early_payment::EarlyPayment;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, Strategy, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: EarlyPayment = EarlyPayment;

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn txn_date() -> Date {
    Date::from_calendar_date(2023, Month::July, 11).unwrap()
}
fn due_date() -> Date {
    Date::from_calendar_date(2023, Month::July, 31).unwrap()
}
fn late_date() -> Date {
    Date::from_calendar_date(2023, Month::August, 1).unwrap()
}

fn money(v: i64) -> Money {
    Money::of(SCALE, MODE, &BigDecimal::from(v))
}
fn installment(principal: i64, interest: i64, fee: i64, penalty: i64) -> Installment {
    Installment::new(
        SCALE,
        MODE,
        due_date(),
        BigDecimal::from(principal),
        BigDecimal::from(interest),
        BigDecimal::from(fee),
        BigDecimal::from(penalty),
    )
}

fn assert_alloc(label: &str, got: &Allocation, p: i64, i: i64, f: i64, pen: i64, unproc: i64) {
    let check = |name: &str, actual: &Money, expected: i64| {
        assert_eq!(
            actual.to_plain_string(),
            money(expected).to_plain_string(),
            "{label}: {name} mismatch"
        );
    };
    check("principal", &got.principal, p);
    check("interest", &got.interest, i);
    check("fee", &got.fee, f);
    check("penalty", &got.penalty, pen);
    check("unprocessed", &got.unprocessed, unproc);
}

// ---- Identity ------------------------------------------------------------------------------------------------------

#[test]
fn strategy_identity() {
    assert_eq!(S.code(), "early-repayment-strategy");
    assert_eq!(S.name(), "Early Repayment Strategy");
}

// ---- On-time: interest → principal → penalty → fee -----------------------------------------------------------------

/// Single component: interest only. inst(P0 I5 F0 Pen0), txn 3 → interest 3.
#[test]
fn on_time_single_interest() {
    let mut inst = installment(0, 5, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_interest", &got, 0, 3, 0, 0, 0);
}

/// Single component: principal only. inst(P5 I0 F0 Pen0), txn 3 → principal 3.
#[test]
fn on_time_single_principal() {
    let mut inst = installment(5, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_principal", &got, 3, 0, 0, 0, 0);
}

/// Combined, exactly covers all four. inst(P3 I2 F2 Pen4), txn 11 → i2, p3, pen4, f2.
#[test]
fn on_time_combined_exact() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("on_time_combined_exact", &got, 3, 2, 2, 4, 0);
}

/// Combined, overpaid: remainder unprocessed. inst(P3 I2 F2 Pen4), txn 20 → i2,p3,pen4,f2, unproc 9.
#[test]
fn on_time_combined_overpaid() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(20)));
    assert_alloc("on_time_combined_overpaid", &got, 3, 2, 2, 4, 9);
}

/// Partial stopping in the first bucket (interest). inst(P3 I5 F2 Pen4), txn 3 → i3.
#[test]
fn on_time_partial_stops_in_interest() {
    let mut inst = installment(3, 5, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_partial_stops_in_interest", &got, 0, 3, 0, 0, 0);
}

/// Partial stopping in principal. inst(P3 I2 F2 Pen4), txn 4 → i2 then p2.
#[test]
fn on_time_partial_stops_in_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_partial_stops_in_principal", &got, 2, 2, 0, 0, 0);
}

/// Partial stopping in penalty. inst(P3 I2 F2 Pen4), txn 6 → i2, p3, pen1.
#[test]
fn on_time_partial_stops_in_penalty() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(6)));
    assert_alloc("on_time_partial_stops_in_penalty", &got, 3, 2, 0, 1, 0);
}

/// Partial stopping in fee (the last bucket). inst(P3 I2 F2 Pen4), txn 10 → i2, p3, pen4, f1.
#[test]
fn on_time_partial_stops_in_fee() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(10)));
    assert_alloc("on_time_partial_stops_in_fee", &got, 3, 2, 1, 4, 0);
}

// ---- Late: delegates to on-time ordering ---------------------------------------------------------------------------

/// Late uses the on-time ordering (the Java late handler delegates to the on-time handler).
#[test]
fn late_uses_on_time_ordering() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_uses_on_time_ordering", &got, 3, 2, 2, 4, 0);
}

/// Late partial stopping in penalty, matching the on-time trace for the same inputs.
#[test]
fn late_partial_stops_in_penalty() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(6)), &[]);
    assert_alloc("late_partial_stops_in_penalty", &got, 3, 2, 0, 1, 0);
}

// ---- In-advance: principal only ----------------------------------------------------------------------------------

/// In-advance partial principal. inst(P3 I2 F2 Pen4), txn 2 → principal 2, everything else untouched.
#[test]
fn in_advance_partial_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(2)), &[]);
    assert_alloc("in_advance_partial_principal", &got, 2, 0, 0, 0, 0);
}

/// In-advance exactly covering principal. inst(P3 I2 F2 Pen4), txn 3 → principal 3, unproc 0.
#[test]
fn in_advance_exact_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(3)), &[]);
    assert_alloc("in_advance_exact_principal", &got, 3, 0, 0, 0, 0);
}

/// In-advance overpay: principal is filled, the rest is unprocessed (interest/fee/penalty are NOT consumed even though
/// the installment owes them — the defining quirk of this strategy). inst(P3 I2 F2 Pen4), txn 10 → principal 3,
/// unproc 7.
#[test]
fn in_advance_overpay_only_principal_consumed() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(10)), &[]);
    assert_alloc(
        "in_advance_overpay_only_principal_consumed",
        &got,
        3,
        0,
        0,
        0,
        7,
    );
}

/// In-advance with no principal outstanding: nothing is allocated even with interest/fee/penalty due, the full amount
/// is unprocessed. inst(P0 I2 F2 Pen4), txn 5 → all zero, unproc 5.
#[test]
fn in_advance_no_principal_outstanding() {
    let mut inst = installment(0, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(5)), &[]);
    assert_alloc("in_advance_no_principal_outstanding", &got, 0, 0, 0, 0, 5);
}
