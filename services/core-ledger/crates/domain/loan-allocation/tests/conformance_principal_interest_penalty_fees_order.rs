//! CHARACTERIZATION tests for `PrincipalInterestPenaltyFeesOrder`
//! (`PrincipalInterestPenaltyFeesOrderLoanRepaymentScheduleTransactionProcessor`).
//!
//! The legacy `PrincipalInterestPenaltyFeesOrderLoanRepaymentScheduleTransactionProcessor` has NO original unit test,
//! so these are NOT captured goldens. The expected values are hand-traced by simulating the (separately golden-verified)
//! `Installment` atoms through the strategy's repayment ordering: principal -> interest -> penalty -> fee, each step
//! consuming `min(remaining, outstanding)`. In-advance and late delegate verbatim to the on-time handler in the Java
//! source, so they are exercised identically here (with the due `charges` slice ignored).
//!
//! Scenarios mirror the input shapes the Due* conformance tests use: single-component; combined P/I/Pen/F; partial
//! payments that stop in each bucket; and in-advance vs on-time vs late timing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::principal_interest_penalty_fees_order::PrincipalInterestPenaltyFeesOrder;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, PendingCharge, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: PrincipalInterestPenaltyFeesOrder = PrincipalInterestPenaltyFeesOrder;

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

// ----- Single-component on-time payments -------------------------------------------------------------------------

#[test]
fn on_time_single_principal() {
    let mut inst = installment(10, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_principal", &got, 5, 0, 0, 0, 0);
}

#[test]
fn on_time_single_interest() {
    let mut inst = installment(0, 10, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_interest", &got, 0, 5, 0, 0, 0);
}

#[test]
fn on_time_single_penalty() {
    let mut inst = installment(0, 0, 0, 10);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_penalty", &got, 0, 0, 0, 5, 0);
}

#[test]
fn on_time_single_fee() {
    let mut inst = installment(0, 0, 10, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("single_fee", &got, 0, 0, 5, 0, 0);
}

// ----- Combined P/I/Pen/F ----------------------------------------------------------------------------------------

#[test]
fn on_time_combined_exact() {
    // P3 I2 F2 Pen4, total 11, txn 11 -> all consumed, nothing left.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("combined_exact", &got, 3, 2, 2, 4, 0);
}

#[test]
fn on_time_combined_overpay() {
    // P3 I2 F2 Pen4, total 11, txn 20 -> all 11 consumed, 9 unprocessed.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(20)));
    assert_alloc("combined_overpay", &got, 3, 2, 2, 4, 9);
}

// ----- Partial payments that stop in each bucket (verifies the principal->interest->penalty->fee order) ----------

#[test]
fn on_time_partial_stops_in_principal() {
    // txn 4 < principal 10 -> only principal partially paid.
    let mut inst = installment(10, 5, 5, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("stops_in_principal", &got, 4, 0, 0, 0, 0);
}

#[test]
fn on_time_partial_stops_in_interest() {
    // P3 consumed, then 2 left of txn 5 lands on interest (out 5).
    let mut inst = installment(3, 5, 5, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("stops_in_interest", &got, 3, 2, 0, 0, 0);
}

#[test]
fn on_time_partial_stops_in_penalty() {
    // P3, I2 consumed (=5), then 3 left of txn 8 lands on penalty (out 5) BEFORE fee.
    let mut inst = installment(3, 2, 5, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(8)));
    assert_alloc("stops_in_penalty", &got, 3, 2, 0, 3, 0);
}

#[test]
fn on_time_partial_stops_in_fee() {
    // P3, I2, Pen4 consumed (=9), then 2 left of txn 11 lands on fee (out 5) LAST.
    let mut inst = installment(3, 2, 5, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("stops_in_fee", &got, 3, 2, 2, 4, 0);
}

// ----- In-advance == on-time (Java in-advance handler delegates to on-time; charges ignored) ---------------------

fn charge(amount: i64, is_penalty: bool, due: Date) -> PendingCharge {
    PendingCharge {
        effective_due_date: Some(due),
        is_penalty,
        amount: BigDecimal::from(amount),
        is_active: true,
        is_pending: true,
    }
}

#[test]
fn in_advance_matches_on_time() {
    // Same shape as combined_exact; due charges supplied but must be ignored (no due-charge ordering for this strategy).
    let charges = [
        charge(2, true, txn_date()),
        charge(1, false, txn_date().previous_day().unwrap()),
    ];
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(
        &S,
        &mut inst,
        &Txn::repayment(txn_date(), money(11)),
        &charges,
    );
    assert_alloc("in_advance_combined", &got, 3, 2, 2, 4, 0);
}

#[test]
fn in_advance_partial_stops_in_penalty() {
    // Same trace as on_time_partial_stops_in_penalty, confirming identical ordering on the in-advance path.
    let mut inst = installment(3, 2, 5, 5);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(8)), &[]);
    assert_alloc("in_advance_stops_in_penalty", &got, 3, 2, 0, 3, 0);
}

// ----- Late == on-time (Java late handler delegates to on-time) -------------------------------------------------

#[test]
fn late_date_is_after_due() {
    assert!(late_date() > due_date());
}

#[test]
fn late_matches_on_time() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_combined", &got, 3, 2, 2, 4, 0);
}

#[test]
fn late_partial_stops_in_interest() {
    let mut inst = installment(3, 5, 5, 5);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(5)), &[]);
    assert_alloc("late_stops_in_interest", &got, 3, 2, 0, 0, 0);
}
