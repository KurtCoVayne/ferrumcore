//! `creocore-strategy` (`CreocoreLoanRepaymentScheduleTransactionProcessor`).
//!
//! For ordinary repayments the else-branch of `handleTransactionThatIsOnTimePaymentOfInstallment` pays components in
//! the order penalty → fee → interest → principal. (The class-level Javadoc says "interest then principal", but the
//! actual `else` block pays penalty and fee first, then interest, then principal — this port reproduces the code, not
//! the comment.) Both the in-advance handler (`handleTransactionThatIsPaymentInAdvanceOfInstallment`) and the late
//! handler (`handleTransactionThatIsALateRepaymentOfInstallment`) delegate verbatim to the on-time handler, so all
//! three timings use the same ordering and pending due charges are ignored.

use super::Strategy;
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Creocore strategy: penalty → fee → interest → principal, identical for on-time, in-advance and late payments.
#[derive(Clone, Copy, Debug, Default)]
pub struct Creocore;

impl Creocore {
    /// The shared repayment ordering used by all three timings (the `else` branch of the Java on-time handler):
    /// penalty → fee → interest → principal, each consuming `min(remaining, outstanding)`.
    fn repay(inst: &mut Installment, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        let fee = inst.pay_fee(&remaining);
        remaining = remaining.minus(&fee);

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

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
}

impl Strategy for Creocore {
    fn code(&self) -> &'static str {
        "creocore-strategy"
    }
    fn name(&self) -> &'static str {
        "Creocore Unique"
    }

    /// On-time ordering: penalty → fee → interest → principal (Java
    /// `handleTransactionThatIsOnTimePaymentOfInstallment`, the `else` branch).
    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        Self::repay(inst, amount)
    }

    /// In-advance ordering: the Java `handleTransactionThatIsPaymentInAdvanceOfInstallment` delegates straight to the
    /// on-time handler, so this uses the same penalty → fee → interest → principal ordering and ignores due charges.
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        _date: Date,
        amount: &Money,
        _charges: &[PendingCharge],
    ) -> Allocation {
        Self::repay(inst, amount)
    }

    /// Late ordering: the Java `handleTransactionThatIsALateRepaymentOfInstallment` also delegates to the on-time
    /// handler. Overridden (rather than left to the trait default) only to mirror the explicit Java delegation.
    fn late_repayment(
        &self,
        inst: &mut Installment,
        _date: Date,
        amount: &Money,
        _charges: &[PendingCharge],
    ) -> Allocation {
        Self::repay(inst, amount)
    }
}
