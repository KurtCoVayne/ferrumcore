//! `early-repayment-strategy` (`EarlyPaymentLoanRepaymentScheduleTransactionProcessor`).
//!
//! On-time and late repayments pay interest → principal → penalty → fee. In-advance ("early") repayments behave
//! distinctively: they allocate the *entire* remainder to **principal only**, leaving interest/fee/penalty untouched
//! and ignoring any due charges. The Java class doc-comment claims "interest first, then principal, penalties and
//! fees" for in-advance, but the actual `handleTransactionThatIsPaymentInAdvanceOfInstallment` else-branch only calls
//! `payPrincipalComponent` — this port reproduces the real code, not the comment.

use super::{Component, Strategy, refund_in_order};
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Early Repayment strategy.
#[derive(Clone, Copy, Debug, Default)]
pub struct EarlyPayment;

impl Strategy for EarlyPayment {
    fn code(&self) -> &'static str {
        "early-repayment-strategy"
    }
    fn name(&self) -> &'static str {
        "Early Repayment Strategy"
    }

    /// On-time ordering: interest → principal → penalty → fee (the `else` branch of
    /// `handleTransactionThatIsOnTimePaymentOfInstallment`).
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

    /// In-advance ordering: principal only (the `else` branch of
    /// `handleTransactionThatIsPaymentInAdvanceOfInstallment`). The entire remainder is offered to principal; interest,
    /// fee and penalty stay zero and the due charges are not consulted. Anything left over after principal is
    /// outstanding is returned unprocessed.
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        _date: Date,
        amount: &Money,
        _charges: &[PendingCharge],
    ) -> Allocation {
        let mut remaining = amount.clone();

        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

        let zero = inst.zero();
        Allocation {
            principal,
            interest: zero.clone(),
            fee: zero.clone(),
            penalty: zero,
            unprocessed: remaining,
        }
    }

    // late_repayment: `handleTransactionThatIsALateRepaymentOfInstallment` delegates to
    // `handleTransactionThatIsOnTimePaymentOfInstallment`, so the default trait impl (on-time ordering) is correct.

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
