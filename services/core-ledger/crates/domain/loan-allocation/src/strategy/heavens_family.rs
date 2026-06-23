//! `heavensfamily-strategy` (`HeavensFamilyLoanRepaymentScheduleTransactionProcessor`).
//!
//! The HeavensFamily processor pays *principal before interest* for standard (on-time / late) repayments, then fee,
//! then penalty. In-advance repayments are special: they pay off principal first, and — uniquely to this strategy — if
//! an installment's entire principal is paid in advance, its interest component is auto-*waived* (not paid) before any
//! over-payment is distributed onward.

use super::{Component, Strategy, refund_in_order};
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The HeavensFamily strategy: standard repayments pay principal → interest → fee → penalty; in-advance repayments pay
/// principal first and auto-waive interest once principal is completed.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeavensFamily;

impl Strategy for HeavensFamily {
    fn code(&self) -> &'static str {
        "heavensfamily-strategy"
    }
    fn name(&self) -> &'static str {
        "HeavensFamily Unique"
    }

    /// On-time repayment ordering — the `else` branch of
    /// `handleTransactionThatIsOnTimePaymentOfInstallment` (Java lines 202-216): "pay of principal before interest",
    /// i.e. principal → interest → fee → penalty.
    ///
    /// The Java late handler (`handleTransactionThatIsALateRepaymentOfInstallment`) delegates straight back to this
    /// on-time handler, so the trait's default `late_repayment` (which calls `on_time_repayment`) is faithful and is
    /// intentionally **not** overridden here.
    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

        let fee = inst.pay_fee(&remaining);
        remaining = remaining.minus(&fee);

        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        Allocation {
            principal,
            interest,
            fee,
            penalty,
            unprocessed: remaining,
        }
    }

    /// In-advance repayment ordering — the `else` branch of
    /// `handleTransactionThatIsPaymentInAdvanceOfInstallment` (Java lines 128-155). Due charges are unused by this
    /// strategy's in-advance path (it never reads the pending-charge list), so `charges` is ignored.
    ///
    /// Faithful reproduction, including the original's quirks:
    /// 1. If the installment's principal is not yet completed (`isPrincipalNotCompleted`), pay principal with the full
    ///    remainder. If that **completes** the principal (`isPrincipalCompleted`), auto-*waive* the installment's
    ///    interest — Java waives `getInterestCharged(...)` (the full charged interest); since interest is untouched at
    ///    this point its outstanding equals its charged amount and `waive_interest` caps at the outstanding, so waiving
    ///    the outstanding is behaviorally identical. The waived amount is discarded (it is **not** reported as the
    ///    interest component). The remainder is then reduced by this first principal payment.
    /// 2. Unconditionally distribute any over-payment: pay principal → interest → fee → penalty.
    ///
    /// QUIRK: the `principalPortion` variable is **reassigned** by the second `payPrincipalComponent` call (Java line
    /// 144), so the reported `principal` component is only the *second-pass* principal payment — the first-pass
    /// principal payment from step 1 is overwritten and never surfaces in `updateComponents`, even though it really did
    /// reduce the installment's outstanding principal. We reproduce that here: `principal` below holds only the
    /// second-pass value.
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        _date: Date,
        amount: &Money,
        _charges: &[PendingCharge],
    ) -> Allocation {
        let mut remaining = amount.clone();

        // Step 1: pay principal first; auto-waive interest if this completes the principal.
        if inst.principal_outstanding().is_greater_than_zero() {
            let first_principal = inst.pay_principal(&remaining);
            if inst.principal_outstanding().is_zero() {
                // Java: waiveInterestComponent(date, getInterestCharged(...)) — the full charged interest. The
                // waiver caps at the interest outstanding (== charged here) and its return value is discarded.
                let interest_charged = inst.interest_outstanding();
                let _waived = inst.waive_interest(&interest_charged);
            }
            remaining = remaining.minus(&first_principal);
        }

        // Step 2: distribute the over-payment. `principal` deliberately captures only this second-pass value,
        // mirroring the Java reassignment of `principalPortion`.
        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

        let fee = inst.pay_fee(&remaining);
        remaining = remaining.minus(&fee);

        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        Allocation {
            principal,
            interest,
            fee,
            penalty,
            unprocessed: remaining,
        }
    }

    /// `isTransactionInAdvanceOfInstallment` override (Java lines 82-98): HeavensFamily decides "in advance" against the
    /// *previous* installment's due date — `installments[index-1]`, or `installments[0]` for the first installment —
    /// rather than the current installment's. This only affects the multi-installment [`crate::driver`] walk.
    fn is_in_advance(&self, index: usize, installments: &[Installment], date: Date) -> bool {
        let previous = index.saturating_sub(1);
        installments
            .get(previous)
            .is_some_and(|i| date < i.due_date)
    }

    /// Refund ordering: penalty → fee → interest → principal (the distinct
    /// `handleRefundTransactionPaymentOfInstallment` of this processor).
    fn refund(&self, inst: &mut Installment, amount: &Money) -> Allocation {
        refund_in_order(
            inst,
            amount,
            [
                Component::Penalty,
                Component::Fee,
                Component::Interest,
                Component::Principal,
            ],
        )
    }
}
