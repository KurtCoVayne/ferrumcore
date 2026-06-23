//! Characterization tests for the `InterestPrincipalPenaltyFeesOrder` strategy
//! (`InterestPrincipalPenaltyFeesOrderLoanRepaymentScheduleTransactionProcessor`).
//!
//! This strategy has NO original Fineract unit test, so the expected values below are NOT captured goldens. They are
//! HAND-TRACED characterizations: each expected tuple was computed by simulating the (already golden-verified)
//! [`Installment`] atoms through the strategy's documented ordering (interest -> principal -> penalty -> fee, each
//! consuming `min(remaining, outstanding)`). They pin the current behaviour and, in particular, the component ORDER.
//!
//! The Java in-advance and late handlers both delegate to the on-time handler, so all three timings are expected to be
//! identical, and the in-advance handler ignores the due charges (see `in_advance_ignores_due_charges`, which would
//! differ under a Due* strategy).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::interest_principal_penalty_fees_order::InterestPrincipalPenaltyFeesOrder;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, PendingCharge, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: InterestPrincipalPenaltyFeesOrder = InterestPrincipalPenaltyFeesOrder;

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

fn charge(amount: i64, is_penalty: bool, due: Date) -> PendingCharge {
    PendingCharge {
        effective_due_date: Some(due),
        is_penalty,
        amount: BigDecimal::from(amount),
        is_active: true,
        is_pending: true,
    }
}

// ----- code / name -------------------------------------------------------------------------------------------------

#[test]
fn code_and_name_match_original() {
    use fc_loan_allocation::strategy::Strategy;
    assert_eq!(S.code(), "interest-principal-penalties-fees-order-strategy");
    assert_eq!(S.name(), "Interest, Principal, Penalties, Fees Order");
}

// ----- single-component on-time (each bucket exercised in isolation) -----------------------------------------------

#[test]
fn single_principal_only() {
    let mut inst = installment(5, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("single_principal_only", &got, 3, 0, 0, 0, 0);
}

#[test]
fn single_interest_only() {
    let mut inst = installment(0, 5, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("single_interest_only", &got, 0, 3, 0, 0, 0);
}

#[test]
fn single_fee_only() {
    let mut inst = installment(0, 0, 5, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("single_fee_only", &got, 0, 0, 3, 0, 0);
}

#[test]
fn single_penalty_only() {
    let mut inst = installment(0, 0, 0, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("single_penalty_only", &got, 0, 0, 0, 3, 0);
}

// ----- combined P/I/F/Pen on-time ---------------------------------------------------------------------------------

/// Full coverage: txn 11 exactly equals total outstanding 3+2+2+4.
/// interest 2 -> principal 3 -> penalty 4 -> fee 2, nothing left over.
#[test]
fn combined_full_payment() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("combined_full_payment", &got, 3, 2, 2, 4, 0);
}

// ----- partial payments that stop in each bucket (these pin the ORDER) --------------------------------------------

/// Stops inside interest: txn 3 < interest outstanding 5. Only interest is touched.
#[test]
fn partial_stops_in_interest() {
    let mut inst = installment(3, 5, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("partial_stops_in_interest", &got, 0, 3, 0, 0, 0);
}

/// Stops inside principal: txn 4 = interest 2 (paid) + principal 2 (partial of 3). Penalty/fee untouched.
#[test]
fn partial_stops_in_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("partial_stops_in_principal", &got, 2, 2, 0, 0, 0);
}

/// Stops inside penalty: txn 7 = interest 2 + principal 3 + penalty 2 (partial of 4). Fee untouched.
/// (Distinguishes this order from the Due* order, which would pay penalty first.)
#[test]
fn partial_stops_in_penalty() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(7)));
    assert_alloc("partial_stops_in_penalty", &got, 3, 2, 0, 2, 0);
}

/// Stops inside fee: txn 9 = interest 2 + principal 3 + penalty 2 + fee 2 (partial of 4).
#[test]
fn partial_stops_in_fee() {
    let mut inst = installment(3, 2, 4, 2);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(9)));
    assert_alloc("partial_stops_in_fee", &got, 3, 2, 2, 2, 0);
}

/// Overpayment: txn 10 against total outstanding 4; each component fully paid, remainder unprocessed.
#[test]
fn overpayment_leaves_unprocessed() {
    let mut inst = installment(1, 1, 1, 1);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(10)));
    assert_alloc("overpayment_leaves_unprocessed", &got, 1, 1, 1, 1, 6);
}

// ----- in-advance vs on-time vs late (all three must agree) -------------------------------------------------------

/// In-advance uses the same ordering as on-time (Java delegates), so the penalty-stop case matches exactly.
#[test]
fn in_advance_matches_on_time() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(7)), &[]);
    assert_alloc("in_advance_matches_on_time", &got, 3, 2, 0, 2, 0);
}

/// Late uses the same ordering as on-time (Java delegates), with a date after the due date.
#[test]
fn late_matches_on_time() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(7)), &[]);
    assert_alloc("late_matches_on_time", &got, 3, 2, 0, 2, 0);
}

/// The in-advance handler ignores the supplied due charges (it delegates straight to on-time). A due penalty charge of
/// 2 present here does NOT divert the allocation: the result is identical to `in_advance_matches_on_time`. (Under a
/// Due*-style strategy this charge would be consumed before interest, producing a different tuple.)
#[test]
fn in_advance_ignores_due_charges() {
    let charges = [charge(2, true, txn_date())];
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(
        &S,
        &mut inst,
        &Txn::repayment(txn_date(), money(7)),
        &charges,
    );
    assert_alloc("in_advance_ignores_due_charges", &got, 3, 2, 0, 2, 0);
}
