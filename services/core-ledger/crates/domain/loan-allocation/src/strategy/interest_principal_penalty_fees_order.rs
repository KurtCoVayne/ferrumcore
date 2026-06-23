//! `interest-principal-penalties-fees-order-strategy`
//! (`InterestPrincipalPenaltyFeesOrderLoanRepaymentScheduleTransactionProcessor`).
//!
//! Repayments pay interest → principal → penalty → fee. The original's in-advance handler
//! (`handleTransactionThatIsPaymentInAdvanceOfInstallment`) and late handler
//! (`handleTransactionThatIsALateRepaymentOfInstallment`) both delegate verbatim to the on-time handler
//! (`handleTransactionThatIsOnTimePaymentOfInstallment`), so all three timings share the same ordering and the
//! in-advance handler ignores the due charges entirely (no due-date cap, no `calculatedPenaltyCharge`/`Fee` pass).

use super::{Component, Strategy, refund_in_order};
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Interest, Principal, Penalties, Fees order strategy.
#[derive(Clone, Copy, Debug, Default)]
pub struct InterestPrincipalPenaltyFeesOrder;

impl Strategy for InterestPrincipalPenaltyFeesOrder {
    fn code(&self) -> &'static str {
        "interest-principal-penalties-fees-order-strategy"
    }
    fn name(&self) -> &'static str {
        "Interest, Principal, Penalties, Fees Order"
    }

    /// On-time repayment ordering — the `else` branch of `handleTransactionThatIsOnTimePaymentOfInstallment`:
    /// interest → principal → penalty → fee, each consuming `min(remaining, outstanding)`.
    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        let fee = inst.pay_fee(&remaining);
        remaining = remaining.minus(&fee);

        Allocation {
            principal,
            interest,
            fee,
            penalty,
            unprocessed: remaining,
        }
    }

    /// In-advance repayment ordering. `handleTransactionThatIsPaymentInAdvanceOfInstallment` delegates straight to the
    /// on-time handler, so the due charges are unused and the ordering is identical to [`Self::on_time_repayment`].
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        date: Date,
        amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation {
        let _ = charges; // the original in-advance handler ignores the due charges
        self.on_time_repayment(inst, date, amount)
    }

    // `late_repayment` is intentionally not overridden: the original
    // `handleTransactionThatIsALateRepaymentOfInstallment` also delegates to the on-time handler, which is exactly the
    // trait's default behaviour.

    /// Refund ordering: fee → penalty → principal → interest (the distinct
    /// `handleRefundTransactionPaymentOfInstallment` of this processor).
    fn refund(&self, inst: &mut Installment, amount: &Money) -> Allocation {
        refund_in_order(
            inst,
            amount,
            [
                Component::Fee,
                Component::Penalty,
                Component::Principal,
                Component::Interest,
            ],
        )
    }
}
