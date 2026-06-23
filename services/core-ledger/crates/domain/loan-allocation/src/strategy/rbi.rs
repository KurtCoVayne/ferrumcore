//! `rbi-india-strategy` (`RBILoanRepaymentScheduleTransactionProcessor`).
//!
//! Adhikar/RBI style. Per RBI regulations all interest (current and overdue) must be paid before principal. For a
//! single installment this resolves to paying penalty → fee → interest → principal:
//!
//! - On-time (`handleTransactionThatIsOnTimePaymentOfInstallment`, else-branch): pay penalty, then fee, then interest,
//!   then principal.
//! - In-advance (`handleTransactionThatIsPaymentInAdvanceOfInstallment`): delegates verbatim to the on-time handler —
//!   so the in-advance ordering is identical (penalty → fee → interest → principal) and the strategy ignores the
//!   pending charges (it never computes due-charge totals).
//! - Late (`handleTransactionThatIsALateRepaymentOfInstallment`, else-branch): a two-loop multi-installment pass that,
//!   reduced to a single installment, pays penalty → fee → interest in the first loop and principal in the second —
//!   behaviorally identical to the on-time ordering, so this strategy keeps the default `late_repayment` delegation.

use super::Strategy;
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The RBI (Adhikar / "Overdue/Due Fee/Int,Principal") strategy: penalty → fee → interest → principal.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rbi;

impl Strategy for Rbi {
    fn code(&self) -> &'static str {
        "rbi-india-strategy"
    }
    fn name(&self) -> &'static str {
        "Overdue/Due Fee/Int,Principal"
    }

    /// Mirrors the else-branch of `handleTransactionThatIsOnTimePaymentOfInstallment`: pay penalty, then fee, then
    /// interest, then principal, decrementing the remaining transaction amount after each component.
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
    /// in-advance ordering is identical and the pending charges are ignored (the RBI processor never reads them).
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

    // late_repayment: the default (delegate to on_time_repayment) is faithful — see the module docs for why the Java
    // two-loop late handler reduces to penalty → fee → interest → principal for a single installment.
}
