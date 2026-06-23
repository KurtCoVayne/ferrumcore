//! Characterization tests for the `rbi-india-strategy` (`RBILoanRepaymentScheduleTransactionProcessor`).
//!
//! This strategy has NO original unit test, so these are CHARACTERIZATION tests (NOT captured goldens): the expected
//! values are hand-traced by simulating the already golden-verified installment atoms through the RBI repayment
//! ordering (penalty → fee → interest → principal). They cover the same input shapes the Due* conformance tests use:
//! single-component payments, a combined P/I/F/Pen payment, partial payments that stop in each bucket, and the
//! in-advance / on-time / late timings.
//!
//! Key faithful behaviors exercised:
//! - On-time / in-advance / late all share the penalty → fee → interest → principal ordering (the Java in-advance
//!   handler delegates to on-time, and the Java late handler's two-loop pass reduces to the same per-installment order).
//! - In-advance IGNORES pending charges entirely (the RBI processor never computes due-charge totals).
//!
//! Currency USD/2, HALF_EVEN, matching the originals.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::rbi::Rbi;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, PendingCharge, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: Rbi = Rbi;

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

// ----- code/name -------------------------------------------------------------------------------------------------

#[test]
fn code_and_name_match_original() {
    use fc_loan_allocation::strategy::Strategy;
    assert_eq!(S.code(), "rbi-india-strategy");
    assert_eq!(S.name(), "Overdue/Due Fee/Int,Principal");
}

// ----- single-component payments (on-time) -----------------------------------------------------------------------

#[test]
fn on_time_single_penalty() {
    let mut inst = installment(0, 0, 0, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_penalty", &got, 0, 0, 0, 5, 0);
}

#[test]
fn on_time_single_fee() {
    let mut inst = installment(0, 0, 5, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_fee", &got, 0, 0, 5, 0, 0);
}

#[test]
fn on_time_single_interest() {
    let mut inst = installment(0, 5, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_interest", &got, 0, 5, 0, 0, 0);
}

#[test]
fn on_time_single_principal() {
    let mut inst = installment(5, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_principal", &got, 5, 0, 0, 0, 0);
}

// ----- combined P/I/F/Pen payments (on-time) -------------------------------------------------------------------

/// Combined P3 I2 F2 Pen4, txn 11 (exact total): penalty 4 → fee 2 → interest 2 → principal 3, nothing left.
#[test]
fn on_time_combined_exact() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("combined_exact", &got, 3, 2, 2, 4, 0);
}

/// Combined P3 I2 F2 Pen4, txn 20 (overpay): all components paid, 9 remains unprocessed.
#[test]
fn on_time_combined_overpay() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(20)));
    assert_alloc("combined_overpay", &got, 3, 2, 2, 4, 9);
}

// ----- partial payments that stop in each bucket (on-time) -----------------------------------------------------

/// Stops in penalty: P3 I2 F2 Pen4, txn 2 → penalty 2 only.
#[test]
fn on_time_partial_stops_in_penalty() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(2)));
    assert_alloc("partial_penalty", &got, 0, 0, 0, 2, 0);
}

/// Stops in fee: txn 5 → penalty 4, then fee 1.
#[test]
fn on_time_partial_stops_in_fee() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("partial_fee", &got, 0, 0, 1, 4, 0);
}

/// Stops in interest: txn 7 → penalty 4, fee 2, then interest 1.
#[test]
fn on_time_partial_stops_in_interest() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(7)));
    assert_alloc("partial_interest", &got, 0, 1, 2, 4, 0);
}

/// Stops in principal: txn 10 → penalty 4, fee 2, interest 2, then principal 2 (of 3 outstanding).
#[test]
fn on_time_partial_stops_in_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(10)));
    assert_alloc("partial_principal", &got, 2, 2, 2, 4, 0);
}

// ----- in-advance: same ordering as on-time, pending charges ignored -------------------------------------------

/// In-advance combined P3 I2 F2 Pen4, txn 11: identical to on-time (penalty → fee → interest → principal). Confirms
/// the in-advance handler does NOT pay principal-only and does NOT special-case due charges.
#[test]
fn in_advance_combined_matches_on_time() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(11)), &[]);
    assert_alloc("in_advance_combined", &got, 3, 2, 2, 4, 0);
}

/// In-advance IGNORES pending charges: a present due penalty charge must not alter the allocation. Same input as the
/// on-time combined-exact case, with a due charge supplied → still penalty 4, fee 2, interest 2, principal 3.
#[test]
fn in_advance_ignores_pending_charges() {
    let charges = [PendingCharge {
        effective_due_date: Some(txn_date()),
        is_penalty: true,
        amount: BigDecimal::from(2),
        is_active: true,
        is_pending: true,
    }];
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(
        &S,
        &mut inst,
        &Txn::repayment(txn_date(), money(11)),
        &charges,
    );
    assert_alloc("in_advance_ignores_charges", &got, 3, 2, 2, 4, 0);
}

// ----- late: same ordering as on-time --------------------------------------------------------------------------

/// Late combined P3 I2 F2 Pen4, txn 11: the Java late two-loop handler reduces to penalty → fee → interest →
/// principal for a single installment, so this matches the on-time combined-exact result.
#[test]
fn late_combined_matches_on_time() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_combined", &got, 3, 2, 2, 4, 0);
}

/// Late partial that stops in interest (before principal) — exercises the RBI "interest before principal" rule on the
/// late path: txn 7 → penalty 4, fee 2, interest 1, principal 0.
#[test]
fn late_partial_interest_before_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(7)), &[]);
    assert_alloc("late_partial_interest", &got, 0, 1, 2, 4, 0);
}
