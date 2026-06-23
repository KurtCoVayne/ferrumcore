//! `principal-interest-penalties-fees-order-strategy`
//! (`PrincipalInterestPenaltyFeesOrderLoanRepaymentScheduleTransactionProcessor`).
//!
//! On-time, in-advance, and late repayments all pay principal → interest → penalty → fee. The Java in-advance handler
//! (`handleTransactionThatIsPaymentInAdvanceOfInstallment`) and late handler
//! (`handleTransactionThatIsALateRepaymentOfInstallment`) both delegate verbatim to the on-time handler
//! (`handleTransactionThatIsOnTimePaymentOfInstallment`), so there is no due-charge ordering: the `charges` slice is
//! unused and the in-advance path is identical to on-time.

use super::{Component, Strategy, refund_in_order};
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Principal, Interest, Penalties, Fees order strategy.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrincipalInterestPenaltyFeesOrder;

impl Strategy for PrincipalInterestPenaltyFeesOrder {
    fn code(&self) -> &'static str {
        "principal-interest-penalties-fees-order-strategy"
    }
    fn name(&self) -> &'static str {
        "Principal, Interest, Penalties, Fees Order"
    }

    /// On-time repayment ordering — the `else` branch of `handleTransactionThatIsOnTimePaymentOfInstallment`:
    /// principal → interest → penalty → fee, each consuming `min(remaining, outstanding)`.
    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

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

    /// In-advance repayment — `handleTransactionThatIsPaymentInAdvanceOfInstallment` delegates verbatim to the on-time
    /// handler, so this is identical to [`Self::on_time_repayment`] and ignores the due `charges`.
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

    // late_repayment is left as the trait default (delegates to on_time_repayment), matching the Java
    // handleTransactionThatIsALateRepaymentOfInstallment, which also delegates to the on-time handler.

    /// Refund ordering: fee → penalty → interest → principal (the distinct
    /// `handleRefundTransactionPaymentOfInstallment` of this processor — note interest before principal, unlike
    /// EarlyPayment / InterestPrincipalPenaltyFeesOrder).
    fn refund(&self, inst: &mut Installment, amount: &Money) -> Allocation {
        refund_in_order(
            inst,
            amount,
            [
                Component::Fee,
                Component::Penalty,
                Component::Interest,
                Component::Principal,
            ],
        )
    }
}
