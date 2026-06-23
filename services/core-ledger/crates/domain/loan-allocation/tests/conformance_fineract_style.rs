//! CHARACTERIZATION tests for the `mifos-standard-strategy`
//! (`FineractStyleLoanRepaymentScheduleTransactionProcessor`).
//!
//! This strategy has NO original unit test, so these are NOT captured goldens. Every expected tuple below is
//! hand-traced by simulating the (already golden-verified, see `conformance_allocation.rs`) `Installment` atoms through
//! the ported ordering: the `else` branch of `handleTransactionThatIsOnTimePaymentOfInstallment` pays
//! penalty → fee → interest → principal, each consuming `min(remaining, outstanding)`; the in-advance and late
//! handlers both delegate to the on-time handler, so all three timings produce identical results for the same inputs.
//!
//! Input shapes mirror those used by the Due* conformance tests: single-component, combined P/I/F/Pen, partial
//! payments that stop in each bucket, overpayment, and on-time vs in-advance vs late timing. Currency USD/2, HALF_EVEN.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::fineract_style::FineractStyle;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, PendingCharge, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: FineractStyle = FineractStyle;

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

// ---- code / name ---------------------------------------------------------------------------------------------------

#[test]
fn code_and_name_match_original() {
    use fc_loan_allocation::Strategy;
    assert_eq!(S.code(), "mifos-standard-strategy");
    assert_eq!(S.name(), "Penalties, Fees, Interest, Principal order");
}

// ---- single-component on-time (exact payment) ----------------------------------------------------------------------

#[test]
fn on_time_penalty_only() {
    // Pen4, txn 4 → penalty 4, nothing else, unprocessed 0.
    let mut inst = installment(0, 0, 0, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_penalty_only", &got, 0, 0, 0, 4, 0);
}

#[test]
fn on_time_fee_only() {
    // F4, txn 4 → fee 4.
    let mut inst = installment(0, 0, 4, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_fee_only", &got, 0, 0, 4, 0, 0);
}

#[test]
fn on_time_interest_only() {
    // I4, txn 4 → interest 4.
    let mut inst = installment(0, 4, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_interest_only", &got, 0, 4, 0, 0, 0);
}

#[test]
fn on_time_principal_only() {
    // P4, txn 4 → principal 4.
    let mut inst = installment(4, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_principal_only", &got, 4, 0, 0, 0, 0);
}

// ---- combined full payment (penalty → fee → interest → principal) -------------------------------------------------

#[test]
fn on_time_combined_full_payment() {
    // P3 I2 F2 Pen4, total outstanding 11, txn 11 → penalty 4, fee 2, interest 2, principal 3, unprocessed 0.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("on_time_combined_full", &got, 3, 2, 2, 4, 0);
}

// ---- partial payments that stop in each bucket --------------------------------------------------------------------

#[test]
fn on_time_partial_stops_in_penalty() {
    // Pen4 F2 I2 P3, txn 3 → penalty consumes 3 (of 4), nothing reaches fee/interest/principal, unprocessed 0.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_partial_penalty", &got, 0, 0, 0, 3, 0);
}

#[test]
fn on_time_partial_stops_in_fee() {
    // Pen4 F2 I2 P3, txn 5 → penalty 4 then fee 1 (of 2), unprocessed 0.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(5)));
    assert_alloc("on_time_partial_fee", &got, 0, 0, 1, 4, 0);
}

#[test]
fn on_time_partial_stops_in_interest() {
    // Pen4 F2 I2 P3, txn 7 → penalty 4, fee 2, interest 1 (of 2), unprocessed 0.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(7)));
    assert_alloc("on_time_partial_interest", &got, 0, 1, 2, 4, 0);
}

#[test]
fn on_time_partial_stops_in_principal() {
    // Pen4 F2 I2 P3, txn 9 → penalty 4, fee 2, interest 2, principal 1 (of 3), unprocessed 0.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(9)));
    assert_alloc("on_time_partial_principal", &got, 1, 2, 2, 4, 0);
}

// ---- overpayment leaves an unprocessed remainder ------------------------------------------------------------------

#[test]
fn on_time_overpayment_leaves_unprocessed() {
    // P3 I2 F2 Pen4, total 11, txn 15 → all four fully paid, unprocessed 4.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(15)));
    assert_alloc("on_time_overpayment", &got, 3, 2, 2, 4, 4);
}

// ---- in-advance and late both delegate to on-time -----------------------------------------------------------------

#[test]
fn in_advance_matches_on_time() {
    // Same inputs as on_time_combined_full, dispatched in-advance (date < due). Charges are ignored (no due pre-pass).
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(11)), &[]);
    assert_alloc("in_advance_combined_full", &got, 3, 2, 2, 4, 0);
}

#[test]
fn in_advance_ignores_due_charges() {
    // A populated due-charge list must not change the result: the Fineract-style in-advance path delegates straight to
    // the on-time handler and never consults `charges`. Pen4 F2 I2 P3, txn 5 → penalty 4, fee 1 (identical to the
    // charge-free partial-fee case).
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
        &Txn::repayment(txn_date(), money(5)),
        &charges,
    );
    assert_alloc("in_advance_ignores_charges", &got, 0, 0, 1, 4, 0);
}

#[test]
fn late_matches_on_time() {
    // Late date (after due) delegates to the on-time handler. Same inputs as on_time_combined_full.
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_combined_full", &got, 3, 2, 2, 4, 0);
}

#[test]
fn late_partial_stops_in_fee() {
    // Late timing, partial payment stopping in fee — identical ordering to on-time. Pen4 F2, txn 5 → penalty 4, fee 1.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(5)), &[]);
    assert_alloc("late_partial_fee", &got, 0, 0, 1, 4, 0);
}
