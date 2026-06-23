//! CHARACTERIZATION tests for the `creocore-strategy` (`CreocoreLoanRepaymentScheduleTransactionProcessor`).
//!
//! The Creocore processor has NO original unit test in Fineract, so these are NOT captured goldens. The expected
//! tuples below were hand-traced by simulating the (already golden-verified) `Installment` atoms through the ported
//! ordering: the `else` branch of `handleTransactionThatIsOnTimePaymentOfInstallment` pays
//! penalty → fee → interest → principal, each step consuming `min(remaining, outstanding)`. The in-advance and late
//! handlers both delegate verbatim to the on-time handler, so all three timings share that ordering and ignore
//! pending due charges.
//!
//! Scenario shapes mirror the Due* conformance tests: single-component, combined P/I/F/Pen, partial payments that
//! stop in each bucket, and on-time vs in-advance vs late timing (all of which must produce identical results here).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::creocore::Creocore;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: Creocore = Creocore;

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

// ----- code / name --------------------------------------------------------------------------------------------------

#[test]
fn code_and_name_match_original() {
    use fc_loan_allocation::Strategy;
    assert_eq!(S.code(), "creocore-strategy");
    assert_eq!(S.name(), "Creocore Unique");
}

// ----- single-component (on-time) -----------------------------------------------------------------------------------

#[test]
fn on_time_penalty_only() {
    // Only penalty outstanding; pays penalty first, nothing else.
    let mut inst = installment(0, 0, 0, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_penalty_only", &got, 0, 0, 0, 4, 0);
}

#[test]
fn on_time_fee_only() {
    let mut inst = installment(0, 0, 3, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_fee_only", &got, 0, 0, 3, 0, 0);
}

#[test]
fn on_time_interest_only() {
    let mut inst = installment(0, 5, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("on_time_interest_only", &got, 0, 5, 0, 0, 0);
}

#[test]
fn on_time_principal_only() {
    let mut inst = installment(7, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(7)));
    assert_alloc("on_time_principal_only", &got, 7, 0, 0, 0, 0);
}

// ----- combined P/I/F/Pen, exact and over-payment -------------------------------------------------------------------

#[test]
fn on_time_full_combined_exact() {
    // P3 I2 F2 Pen4, txn 11 = exact total. Order penalty(4) → fee(2) → interest(2) → principal(3).
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("on_time_full_combined_exact", &got, 3, 2, 2, 4, 0);
}

#[test]
fn on_time_overpayment_leaves_unprocessed() {
    // Total 11, txn 15 → all four buckets filled, 4 left unprocessed.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(15)));
    assert_alloc("on_time_overpayment", &got, 3, 2, 2, 4, 4);
}

// ----- partial payments that stop in each bucket --------------------------------------------------------------------

#[test]
fn on_time_partial_stops_in_penalty() {
    // P3 I2 F2 Pen4, txn 3 → penalty consumes 3, stops there.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_partial_penalty", &got, 0, 0, 0, 3, 0);
}

#[test]
fn on_time_partial_stops_in_fee() {
    // txn 5 → penalty 4, then fee 1 (of 2), stops in fee.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("on_time_partial_fee", &got, 0, 0, 1, 4, 0);
}

#[test]
fn on_time_partial_stops_in_interest() {
    // txn 7 → penalty 4, fee 2, then interest 1 (of 2), stops in interest.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(7)));
    assert_alloc("on_time_partial_interest", &got, 0, 1, 2, 4, 0);
}

#[test]
fn on_time_partial_stops_in_principal() {
    // txn 10 → penalty 4, fee 2, interest 2, then principal 2 (of 3), stops in principal.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(10)));
    assert_alloc("on_time_partial_principal", &got, 2, 2, 2, 4, 0);
}

// ----- in-advance == on-time (delegates) ----------------------------------------------------------------------------

#[test]
fn in_advance_matches_on_time_full() {
    // In-advance handler delegates to on-time; due charges (passed empty here) are ignored regardless.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(11)), &[]);
    assert_alloc("in_advance_full", &got, 3, 2, 2, 4, 0);
}

#[test]
fn in_advance_matches_on_time_partial_fee() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(5)), &[]);
    assert_alloc("in_advance_partial_fee", &got, 0, 0, 1, 4, 0);
}

// ----- late == on-time (delegates) ----------------------------------------------------------------------------------

#[test]
fn late_matches_on_time_full() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_full", &got, 3, 2, 2, 4, 0);
}

#[test]
fn late_matches_on_time_partial_interest() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(7)), &[]);
    assert_alloc("late_partial_interest", &got, 0, 1, 2, 4, 0);
}

// ----- all three timings agree --------------------------------------------------------------------------------------

#[test]
fn all_timings_produce_identical_result() {
    let txn_amt = money(8); // penalty 4, fee 2, interest 2 → (0, 2, 2, 4, 0)
    let mut a = installment(3, 2, 2, 4);
    let mut b = installment(3, 2, 2, 4);
    let mut c = installment(3, 2, 2, 4);
    let ot = allocate_on_time(&S, &mut a, &Txn::repayment(txn_date(), txn_amt.clone()));
    let ia = allocate_in_advance(
        &S,
        &mut b,
        &Txn::repayment(txn_date(), txn_amt.clone()),
        &[],
    );
    let la = allocate_late(&S, &mut c, &Txn::repayment(late_date(), txn_amt), &[]);
    assert_alloc("on_time", &ot, 0, 2, 2, 4, 0);
    assert_alloc("in_advance", &ia, 0, 2, 2, 4, 0);
    assert_alloc("late", &la, 0, 2, 2, 4, 0);
}
