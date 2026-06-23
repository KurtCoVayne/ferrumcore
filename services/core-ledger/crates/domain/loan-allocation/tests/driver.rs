//! CHARACTERIZATION tests for the multi-installment driver
//! (`AbstractLoanRepaymentScheduleTransactionProcessor.processTransaction`'s installment loop + the
//! `processLatestTransaction` overpayment leftover).
//!
//! These are CHARACTERIZATION tests, not conformance tests: the original Java has no single self-contained unit test
//! that pins the *cross-installment* walk for the `DuePenIntPriFee` strategy with the exact tuples below, so the
//! expected values here were derived by hand-tracing the ported driver + strategy together. They lock in the current
//! behavior (an early payment spilling across installments, a late payment leaving an overpayment, and an exact payoff)
//! so future refactors are caught. Uncertainties are flagged in the crate notes; the single-installment per-component
//! arithmetic itself is already pinned by the VERIFIED conformance suite in `conformance_allocation.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::driver::{DriveResult, drive};
use fc_loan_allocation::strategy::due_pen_int_pri_fee::DuePenIntPriFee;
use fc_loan_allocation::strategy::heavens_family::HeavensFamily;
use fc_loan_allocation::{Allocation, Installment, Txn};
use fc_money::Money;
use time::{Date, Month};

const S: DuePenIntPriFee = DuePenIntPriFee;

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn date(y: i32, m: Month, d: u8) -> Date {
    Date::from_calendar_date(y, m, d).unwrap()
}
fn money(v: i64) -> Money {
    Money::of(SCALE, MODE, &BigDecimal::from(v))
}
fn installment(due: Date, principal: i64, interest: i64, fee: i64, penalty: i64) -> Installment {
    Installment::new(
        SCALE,
        MODE,
        due,
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

fn assert_final_unprocessed(label: &str, res: &DriveResult, expected: i64) {
    assert_eq!(
        res.unprocessed.to_plain_string(),
        money(expected).to_plain_string(),
        "{label}: final unprocessed (overpayment) mismatch"
    );
}

/// Early ("in advance") payment that spills across two future installments.
///
/// Txn 2023-07-11 (before both due dates) for 15. Inst1 (due 07-31) and Inst2 (due 08-31) are each P3 I2 F2 Pen4
/// (total outstanding 11). The strategy's in-advance pass pays Inst1 in full (penalty→interest→principal→fee = 11) and
/// carries the 4 remainder onto Inst2, where it covers only the penalty. Nothing is left over.
#[test]
fn early_payment_spills_across_installments() {
    let mut installments = [
        installment(date(2023, Month::July, 31), 3, 2, 2, 4),
        installment(date(2023, Month::August, 31), 3, 2, 2, 4),
    ];
    let txn = Txn::repayment(date(2023, Month::July, 11), money(15));

    let res = drive(&S, &mut installments, &txn, &[]);

    assert_eq!(
        res.allocations.len(),
        2,
        "both installments should be touched"
    );
    let (i0, a0) = &res.allocations[0];
    let (i1, a1) = &res.allocations[1];
    assert_eq!(*i0, 0);
    assert_eq!(*i1, 1);
    // Inst1 fully paid; 4 carried forward as the handler's returned remainder.
    assert_alloc("spill: inst1", a0, 3, 2, 2, 4, 4);
    // Inst2 receives the 4: in-advance ordering pays penalty first.
    assert_alloc("spill: inst2", a1, 0, 0, 0, 4, 0);
    assert_final_unprocessed("spill", &res, 0);
}

/// A late payment (txn after the only installment's due date) that overpays, leaving an overpayment remainder.
///
/// Txn 2023-08-01 (after due 07-31) for 15 against a single installment P3 I2 F2 Pen4 (total 11). Late delegates to the
/// on-time ordering: penalty 4 → interest 2 → principal 3 → fee 2 = 11 consumed, 4 left over as the loan overpayment.
#[test]
fn late_payment_with_overpayment() {
    let mut installments = [installment(date(2023, Month::July, 31), 3, 2, 2, 4)];
    let txn = Txn::repayment(date(2023, Month::August, 1), money(15));

    let res = drive(&S, &mut installments, &txn, &[]);

    assert_eq!(res.allocations.len(), 1);
    let (idx, a) = &res.allocations[0];
    assert_eq!(*idx, 0);
    assert_alloc("late: inst", a, 3, 2, 2, 4, 4);
    assert_final_unprocessed("late", &res, 4);
}

/// Exact payoff across two installments with a mixed timing decision per installment.
///
/// Txn 2023-07-31 equals Inst1's due date (→ on-time for Inst1) but precedes Inst2's due 08-31 (→ in-advance for
/// Inst2). Each installment is P3 I2 F2 Pen4 (total 11); txn 22 exactly clears both. The driver routes the 11 remainder
/// from Inst1 into Inst2's in-advance handler, ending with zero unprocessed.
#[test]
fn exact_payoff_across_two_installments() {
    let mut installments = [
        installment(date(2023, Month::July, 31), 3, 2, 2, 4),
        installment(date(2023, Month::August, 31), 3, 2, 2, 4),
    ];
    let txn = Txn::repayment(date(2023, Month::July, 31), money(22));

    let res = drive(&S, &mut installments, &txn, &[]);

    assert_eq!(res.allocations.len(), 2);
    let (i0, a0) = &res.allocations[0];
    let (i1, a1) = &res.allocations[1];
    assert_eq!(*i0, 0);
    assert_eq!(*i1, 1);
    // Inst1 on-time: penalty→interest→principal→fee, fully paid, 11 carried forward.
    assert_alloc("payoff: inst1", a0, 3, 2, 2, 4, 11);
    // Inst2 in-advance: the 11 remainder clears it exactly.
    assert_alloc("payoff: inst2", a1, 3, 2, 2, 4, 0);
    assert_final_unprocessed("payoff", &res, 0);
}

/// A fully-paid-off installment is skipped (its total outstanding is zero), so the money flows to the next one.
///
/// Inst1 is constructed paid off (all-zero charged amounts → zero total outstanding → `isNotFullyPaidOff()` is false);
/// Inst2 (due 08-31, P3 I2 F2 Pen4) receives the whole in-advance payment. This pins the `isNotFullyPaidOff` guard.
#[test]
fn skips_fully_paid_installment() {
    let mut installments = [
        installment(date(2023, Month::July, 31), 0, 0, 0, 0),
        installment(date(2023, Month::August, 31), 3, 2, 2, 4),
    ];
    let txn = Txn::repayment(date(2023, Month::July, 11), money(11));

    let res = drive(&S, &mut installments, &txn, &[]);

    // Only the second installment is touched; the first is skipped.
    assert_eq!(res.allocations.len(), 1);
    let (idx, a) = &res.allocations[0];
    assert_eq!(
        *idx, 1,
        "the skipped first installment leaves index 1 as the only touched one"
    );
    assert_alloc("skip: inst2", a, 3, 2, 2, 4, 0);
    assert_final_unprocessed("skip", &res, 0);
}

/// Regression: HeavensFamily decides "in advance" against the *previous* installment's due date, so the timing bucket
/// for a later installment can differ from the base rule (`AbstractLoanRepaymentScheduleTransactionProcessor`
/// `isTransactionInAdvanceOfInstallment` override, Java lines 82-98). This is the divergence the workflow's adversarial
/// verifier flagged in the first-cut driver.
///
/// Inst1 (due 07-31) is paid off and skipped; Inst2 (index 1, due 08-31, P5 I3) is paid by an 8 on 2023-08-15. The
/// previous installment (Inst1) is due 07-31, and 08-15 is NOT before 07-31, so Inst2 is treated **on-time** (not
/// in-advance): HeavensFamily on-time pays principal 5 → interest 3 → (5, 3, 0, 0). Under the (incorrect) base rule
/// Inst2 would be "in advance" — its principal-first/auto-waive path would yield (0, 0, 0, 0) with 3 unprocessed — so
/// this asserts the override is honored end-to-end through the driver.
#[test]
fn heavens_family_in_advance_uses_previous_installment_due_date() {
    let mut installments = [
        installment(date(2023, Month::July, 31), 0, 0, 0, 0),
        installment(date(2023, Month::August, 31), 5, 3, 0, 0),
    ];
    let txn = Txn::repayment(date(2023, Month::August, 15), money(8));

    let res = drive(&HeavensFamily, &mut installments, &txn, &[]);

    assert_eq!(res.allocations.len(), 1);
    let (idx, a) = &res.allocations[0];
    assert_eq!(*idx, 1);
    assert_alloc("heavens prev-due: inst2 on-time", a, 5, 3, 0, 0, 0);
    assert_final_unprocessed("heavens prev-due", &res, 0);
}
