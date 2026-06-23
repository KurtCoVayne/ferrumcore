//! `mifos-standard-strategy` (`FineractStyleLoanRepaymentScheduleTransactionProcessor`).
//!
//! The original "old style" processor: for ALL timings (on-time, in-advance, late) it pays off components in the same
//! fixed order — penalty → fee → interest → principal. Its `handleTransactionThatIsPaymentInAdvanceOfInstallment` and
//! `handleTransactionThatIsALateRepaymentOfInstallment` both delegate straight to
//! `handleTransactionThatIsOnTimePaymentOfInstallment`, so the in-advance and late paths reuse the on-time ordering
//! verbatim (no due-charge pre-pass, no late-specific path).
//!
//! Note: the original `STRATEGY_NAME` is "Penalties, Fees, Interest, Principal order", which matches the else-branch
//! ordering (penalty → fee → interest → principal) — the class javadoc line "pays off interest first, then principal"
//! is stale documentation and does NOT reflect the actual code, which is what we port.

use super::Strategy;
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Fineract "old style" / mifos-standard strategy: penalty → fee → interest → principal for every timing.
#[derive(Clone, Copy, Debug, Default)]
pub struct FineractStyle;

impl Strategy for FineractStyle {
    fn code(&self) -> &'static str {
        "mifos-standard-strategy"
    }
    fn name(&self) -> &'static str {
        "Penalties, Fees, Interest, Principal order"
    }

    /// The `else` branch of `handleTransactionThatIsOnTimePaymentOfInstallment`: pay penalty, then fee, then interest,
    /// then principal, each consuming `min(remaining, outstanding)`. `transactionAmountRemaining` is the unprocessed
    /// remainder returned by the original handler.
    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        // payPenaltyChargesComponent
        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        // payFeeChargesComponent
        let fee = inst.pay_fee(&remaining);
        remaining = remaining.minus(&fee);

        // payInterestComponent
        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

        // payPrincipalComponent
        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        Allocation {
            principal,
            interest,
            fee,
            penalty,
            unprocessed: remaining,
        }
    }

    /// `handleTransactionThatIsPaymentInAdvanceOfInstallment` delegates verbatim to the on-time handler, so the
    /// in-advance ordering is identical (the `charges` are unused — there is no due-charge pre-pass here).
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        date: Date,
        amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation {
        let _ = charges;
        self.on_time_repayment(inst, date, amount)
    }

    // late_repayment intentionally uses the trait default, which calls on_time_repayment — faithful to
    // handleTransactionThatIsALateRepaymentOfInstallment delegating to the on-time handler.
}
