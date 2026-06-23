//! Characterization tests for the `heavensfamily-strategy`
//! (`HeavensFamilyLoanRepaymentScheduleTransactionProcessor`).
//!
//! IMPORTANT: this strategy has NO original Fineract unit test, so these are **characterization** tests, not captured
//! goldens. Each expected tuple was hand-traced by simulating the (already golden-verified) `Installment` atoms through
//! the ported ordering. They pin the current faithful behavior — including the strategy's documented quirks — rather
//! than independently-verified reference output.
//!
//! Ordering under test (from the Java):
//! - on-time / late: principal -> interest -> fee -> penalty ("pay of principal before interest").
//! - in-advance: pay principal first; if that completes the principal, auto-WAIVE the interest (not pay it); then
//!   distribute any over-payment principal -> interest -> fee -> penalty. The reported `principal` component is only
//!   the *second-pass* principal payment (the Java `principalPortion` reassignment quirk).
//!
//! Currency is USD/2 (scale 2, HalfEven), matching the Due* conformance tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::heavens_family::HeavensFamily;
use fc_loan_allocation::strategy::{allocate_in_advance, allocate_late, allocate_on_time};
use fc_loan_allocation::{Allocation, Installment, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: HeavensFamily = HeavensFamily;

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

// ----- metadata -------------------------------------------------------------------------------------------------

#[test]
fn code_and_name() {
    use fc_loan_allocation::strategy::Strategy;
    assert_eq!(S.code(), "heavensfamily-strategy");
    assert_eq!(S.name(), "HeavensFamily Unique");
}

// ----- on-time: single component --------------------------------------------------------------------------------
// Ordering: principal -> interest -> fee -> penalty.

#[test]
fn on_time_single_principal() {
    let mut inst = installment(5, 0, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_principal", &got, 3, 0, 0, 0, 0);
}

#[test]
fn on_time_single_interest() {
    let mut inst = installment(0, 5, 0, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_interest", &got, 0, 3, 0, 0, 0);
}

#[test]
fn on_time_single_fee() {
    let mut inst = installment(0, 0, 5, 0);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_fee", &got, 0, 0, 3, 0, 0);
}

#[test]
fn on_time_single_penalty() {
    let mut inst = installment(0, 0, 0, 5);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(3)));
    assert_alloc("on_time_single_penalty", &got, 0, 0, 0, 3, 0);
}

// ----- on-time: combined and partial (stops in each bucket) -----------------------------------------------------
// Installment P3 I2 F2 Pen4.

#[test]
fn on_time_combined_exact() {
    // 11 = 3 + 2 + 2 + 4, nothing left over.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(11)));
    assert_alloc("on_time_combined_exact", &got, 3, 2, 2, 4, 0);
}

#[test]
fn on_time_partial_stops_in_principal() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(2)));
    assert_alloc("on_time_partial_stops_in_principal", &got, 2, 0, 0, 0, 0);
}

#[test]
fn on_time_partial_stops_in_interest() {
    // 4 = principal 3 + interest 1.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(4)));
    assert_alloc("on_time_partial_stops_in_interest", &got, 3, 1, 0, 0, 0);
}

#[test]
fn on_time_partial_stops_in_fee() {
    // 6 = principal 3 + interest 2 + fee 1.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(6)));
    assert_alloc("on_time_partial_stops_in_fee", &got, 3, 2, 1, 0, 0);
}

#[test]
fn on_time_partial_stops_in_penalty() {
    // 8 = principal 3 + interest 2 + fee 2 + penalty 1.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(8)));
    assert_alloc("on_time_partial_stops_in_penalty", &got, 3, 2, 2, 1, 0);
}

#[test]
fn on_time_overpayment_leaves_unprocessed() {
    // 20 = 11 due + 9 left over.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_on_time(&S, &mut inst, &Txn::repayment(txn_date(), money(20)));
    assert_alloc(
        "on_time_overpayment_leaves_unprocessed",
        &got,
        3,
        2,
        2,
        4,
        9,
    );
}

// ----- late: delegates to on-time (Java handleTransactionThatIsALateRepaymentOfInstallment) ---------------------

#[test]
fn late_matches_on_time_ordering() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(8)), &[]);
    // identical to on_time_partial_stops_in_penalty.
    assert_alloc("late_matches_on_time_ordering", &got, 3, 2, 2, 1, 0);
}

#[test]
fn late_combined_exact() {
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_late(&S, &mut inst, &Txn::repayment(late_date(), money(11)), &[]);
    assert_alloc("late_combined_exact", &got, 3, 2, 2, 4, 0);
}

// ----- in-advance -----------------------------------------------------------------------------------------------
// QUIRK 1: pay principal first; if principal becomes complete, AUTO-WAIVE interest (interest is not reported as paid).
// QUIRK 2: the reported `principal` component is only the SECOND-pass principal payment (Java reassigns
//          principalPortion at line 144), so the first-pass principal payment never surfaces in the tuple even though
//          it really reduced the outstanding principal.

#[test]
fn in_advance_partial_principal_not_completed() {
    // P5 I2 F2 Pen4, txn 3. Step1: pay principal 3 (outstanding now 2, NOT completed -> no waive), remaining 0.
    // Step2: nothing left. Reported principal is the *second-pass* value (0) -> the actual 3 paid is invisible here.
    let mut inst = installment(5, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(3)), &[]);
    assert_alloc(
        "in_advance_partial_principal_not_completed",
        &got,
        0,
        0,
        0,
        0,
        0,
    );
}

#[test]
fn in_advance_principal_completed_interest_waived() {
    // P3 I2 F2 Pen4, txn 5. Step1: pay principal 3 (completes) -> waive interest 2; remaining 5-3=2.
    // Step2: principal 0, interest 0 (already waived), fee 2, penalty 0; remaining 0.
    // Reported: principal 0 (second pass), interest 0 (waived, not paid), fee 2.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(5)), &[]);
    assert_alloc(
        "in_advance_principal_completed_interest_waived",
        &got,
        0,
        0,
        2,
        0,
        0,
    );
}

#[test]
fn in_advance_principal_completed_exact() {
    // P3 I2 F2 Pen4, txn 3. Step1: pay principal 3 (completes) -> waive interest 2; remaining 0.
    // Step2: everything 0. Reported all zero -> the entire principal payment is invisible in the tuple.
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(3)), &[]);
    assert_alloc("in_advance_principal_completed_exact", &got, 0, 0, 0, 0, 0);
}

#[test]
fn in_advance_principal_completed_then_overpay_fee_and_penalty() {
    // P3 I2 F2 Pen4, txn 12. Step1: pay principal 3 (completes) -> waive interest 2; remaining 9.
    // Step2: principal 0, interest 0, fee 2 (remaining 7), penalty 4 (remaining 3).
    let mut inst = installment(3, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(12)), &[]);
    assert_alloc(
        "in_advance_principal_completed_then_overpay_fee_and_penalty",
        &got,
        0,
        0,
        2,
        4,
        3,
    );
}

#[test]
fn in_advance_zero_principal_no_waive() {
    // P0 I2 F2 Pen4, txn 5. Step1: principal outstanding 0 -> whole block skipped (NO interest waive).
    // Step2: principal 0, interest 2 (remaining 3), fee 2 (remaining 1), penalty 1 (remaining 0).
    let mut inst = installment(0, 2, 2, 4);
    let got = allocate_in_advance(&S, &mut inst, &Txn::repayment(txn_date(), money(5)), &[]);
    assert_alloc("in_advance_zero_principal_no_waive", &got, 0, 2, 2, 1, 0);
}

#[test]
fn in_advance_charges_argument_is_ignored() {
    // The in-advance handler never reads the pending-charge list; passing some must not change the result. Same
    // inputs as in_advance_zero_principal_no_waive.
    let charges = [fc_loan_allocation::PendingCharge {
        effective_due_date: Some(txn_date()),
        is_penalty: true,
        amount: BigDecimal::from(99),
        is_active: true,
        is_pending: true,
    }];
    let mut inst = installment(0, 2, 2, 4);
    let got = allocate_in_advance(
        &S,
        &mut inst,
        &Txn::repayment(txn_date(), money(5)),
        &charges,
    );
    assert_alloc(
        "in_advance_charges_argument_is_ignored",
        &got,
        0,
        2,
        2,
        1,
        0,
    );
}
