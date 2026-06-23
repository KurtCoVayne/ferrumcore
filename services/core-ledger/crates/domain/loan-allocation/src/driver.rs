//! Multi-installment driver — a faithful port of the installment-walk in
//! `AbstractLoanRepaymentScheduleTransactionProcessor.processTransaction` (the loop) plus the overpayment leftover of
//! `processLatestTransaction`.
//!
//! Given an *ordered* list of installments and a single repayment-like transaction, the original walks the installments
//! in schedule order and, for each one that still has an outstanding balance, routes the still-unprocessed amount into
//! the in-advance / on-time / late timing bucket (decided per installment by the transaction date vs that installment's
//! due date), carrying the handler's returned remainder forward to the next installment. Whatever is left after the last
//! installment is the transaction's overpayment.
//!
//! This module models only the *pure* allocation walk. Everything the original loop touches that is persistence,
//! JPA-mapping, charge bookkeeping, or overpayment-account routing (the `LoanTransactionToRepaymentScheduleMapping`
//! list, `loanTransaction.updateLoanTransactionToRepaymentScheduleMappings`, `onLoanOverpayment`,
//! `setOverPayments`, the overpayment holder, the waiver component-zeroing in `processLatestTransaction`) is out of
//! scope, exactly as the task requires.

use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge, Txn};
use crate::strategy::{Strategy, allocate_in_advance, allocate_late, allocate_on_time};
use fc_money::Money;

/// The outcome of driving one transaction across an ordered installment list: the per-installment allocations (in
/// installment order, one entry for *every* installment the walk produced an [`Allocation`] for) and the final
/// `unprocessed` remainder — the original's leftover `transactionAmountUnprocessed`, i.e. the loan overpayment.
#[derive(Clone, Debug)]
pub struct DriveResult {
    /// One `(index, allocation)` pair per installment that the walk actually touched, in installment (schedule) order.
    /// `index` is the position in the input slice. Installments the walk skipped (already fully paid off, or reached
    /// after the money ran out) produce no entry — mirroring the original, which only calls a handler (and thus only
    /// produces `updateComponents` movement) for touched installments.
    pub allocations: Vec<(usize, Allocation)>,
    /// The amount left after the last installment — `processTransaction`'s returned `transactionAmountUnprocessed`,
    /// which `processLatestTransaction` then treats as the overpayment.
    pub unprocessed: Money,
}

/// `isNotFullyPaidOff()` for the pure model.
///
/// The original returns `!obligationsMet`, and `obligationsMet` is itself derived (`updateObligationMet`) from
/// `getTotalOutstanding(currency).isZero()`. Without the persisted obligation flag, the faithful behavioral equivalent
/// is therefore "the installment still has a non-zero total outstanding". Total outstanding mirrors
/// `LoanRepaymentScheduleInstallment.getTotalOutstanding`: principal + interest + fee + penalty outstanding.
fn is_not_fully_paid_off(inst: &Installment) -> bool {
    let total = inst
        .principal_outstanding()
        .plus(&inst.interest_outstanding())
        .plus(&inst.fee_charges_outstanding())
        .plus(&inst.penalty_charges_outstanding());
    total.is_greater_than_zero()
}

/// Drive a single repayment-like transaction across the ordered `installments`, returning the per-installment
/// allocations and the final unprocessed overpayment.
///
/// This is `processTransaction`'s installment loop (lines `for (... currentInstallment : installments)`): the
/// transaction starts with its full amount unprocessed; each installment that is still `isGreaterThanZero` unprocessed
/// *and* `isNotFullyPaidOff` is routed through the timing-bucket dispatcher
/// (`isTransactionInAdvanceOfInstallment` → in-advance, else `isTransactionALateRepaymentOnInstallment` → late, else
/// on-time), whose returned remainder becomes the unprocessed amount for the next installment.
///
/// The caller is responsible for passing the installments in the order the original sorts them (by due date) — this
/// function walks them as given, exactly like the original loop over the already-sorted list.
#[must_use]
pub fn drive(
    strategy: &dyn Strategy,
    installments: &mut [Installment],
    txn: &Txn,
    charges: &[PendingCharge],
) -> DriveResult {
    // `transactionAmountUnprocessed = loanTransaction.getAmount(currency)` (no `amountToProcess` override: the
    // charge-payment split path of `processTransaction` is out of scope for this pure repayment driver).
    let mut unprocessed = txn.amount.clone();
    let mut allocations: Vec<(usize, Allocation)> = Vec::new();

    for index in 0..installments.len() {
        // `if (transactionAmountUnprocessed.isGreaterThanZero())` — once the money is exhausted the original keeps
        // iterating but does nothing, so we mirror that by simply not touching further installments.
        if !unprocessed.is_greater_than_zero() {
            continue;
        }
        // `if (currentInstallment.isNotFullyPaidOff())` — skip installments with no outstanding balance.
        if !is_not_fully_paid_off(&installments[index]) {
            continue;
        }

        // Per-installment timing decision, exactly as `processTransaction` branches — but delegated to the strategy's
        // timing predicates, because they are *strategy-overridable* and index-aware (HeavensFamily decides "in advance"
        // against the previous installment's due date, not the current one):
        //   isTransactionInAdvanceOfInstallment → `allocate_in_advance`
        //   else isTransactionALateRepaymentOnInstallment → `allocate_late`
        //   else (on the due date) → `allocate_on_time`.
        // The predicates need an immutable view of the whole slice; resolve them before taking the mutable element
        // borrow below.
        let in_advance = strategy.is_in_advance(index, installments, txn.date);
        let late = strategy.is_late(index, installments, txn.date);

        // Build a fresh `Txn` carrying the still-unprocessed amount because the dispatchers read `txn.amount`; the date
        // and kind are preserved so the common waiver / charge-payment branches stay faithful.
        let step_txn = Txn {
            amount: unprocessed.clone(),
            date: txn.date,
            kind: txn.kind.clone(),
        };
        let inst = &mut installments[index];
        let alloc = if in_advance {
            allocate_in_advance(strategy, inst, &step_txn, charges)
        } else if late {
            allocate_late(strategy, inst, &step_txn, charges)
        } else {
            allocate_on_time(strategy, inst, &step_txn)
        };

        // The handler's returned remainder becomes the unprocessed amount carried to the next installment
        // (`transactionAmountUnprocessed = handleTransaction...(...)`).
        unprocessed = alloc.unprocessed.clone();
        allocations.push((index, alloc));
    }

    DriveResult {
        allocations,
        unprocessed,
    }
}
