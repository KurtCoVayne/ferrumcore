//! `due-penalty-interest-principal-fee-in-advance-penalty-interest-principal-fee-strategy`
//! (`DuePenIntPriFeeInAdvancePenIntPriFeeLoanRepaymentScheduleTransactionProcessor`).
//!
//! On-time and late repayments pay penalty → interest → principal → fee. In-advance repayments first pay the *due*
//! charges, then re-run (without the due-date cap) to distribute the in-advance penalty → interest → principal → fee.

use super::{Strategy, due_charge_totals};
use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge};
use fc_money::Money;
use time::Date;

/// The Due penalty/interest/principal/fee, in-advance penalty/interest/principal/fee strategy.
#[derive(Clone, Copy, Debug, Default)]
pub struct DuePenIntPriFee;

impl Strategy for DuePenIntPriFee {
    fn code(&self) -> &'static str {
        "due-penalty-interest-principal-fee-in-advance-penalty-interest-principal-fee-strategy"
    }
    fn name(&self) -> &'static str {
        "Due penalty, interest, principal, fee, In advance penalty, interest, principal, fee"
    }

    fn on_time_repayment(&self, inst: &mut Installment, _date: Date, amount: &Money) -> Allocation {
        let mut remaining = amount.clone();

        let penalty = inst.pay_penalty(&remaining);
        remaining = remaining.minus(&penalty);

        let interest = inst.pay_interest(&remaining);
        remaining = remaining.minus(&interest);

        let principal = inst.pay_principal(&remaining);
        remaining = remaining.minus(&principal);

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

    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        date: Date,
        amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation {
        let mut remaining = amount.clone();
        let mut penalty = inst.zero();
        let mut interest = inst.zero();
        let mut principal = inst.zero();
        let mut fee = inst.zero();

        let (mut calc_penalty, mut calc_fee) = due_charge_totals(inst, date, charges);
        let due_date = inst.due_date;

        let mut ignore_due_date_check = false;
        let mut rerun = false;
        loop {
            // Penalty: capped at the due total on the first pass, at the full remainder thereafter.
            if ignore_due_date_check || calc_penalty.is_greater_than(&remaining) {
                calc_penalty = remaining.clone();
            }
            let sub = inst.pay_penalty(&calc_penalty);
            remaining = remaining.minus(&sub);
            penalty = penalty.plus(&sub);

            // Interest + principal: only once the due-date cap is dropped, or for a due/late payment.
            if ignore_due_date_check || date >= due_date {
                let si = inst.pay_interest(&remaining);
                remaining = remaining.minus(&si);
                interest = interest.plus(&si);

                let sp = inst.pay_principal(&remaining);
                remaining = remaining.minus(&sp);
                principal = principal.plus(&sp);
            }

            // Fee: capped at the due total on the first pass, at the full remainder thereafter.
            if ignore_due_date_check || calc_fee.is_greater_than(&remaining) {
                calc_fee = remaining.clone();
            }
            let sf = inst.pay_fee(&calc_fee);
            remaining = remaining.minus(&sf);
            fee = fee.plus(&sf);

            if remaining.is_greater_than_zero() {
                ignore_due_date_check = true;
            }
            rerun = !rerun;
            if !(ignore_due_date_check && rerun) {
                break;
            }
        }

        Allocation {
            principal,
            interest,
            fee,
            penalty,
            unprocessed: remaining,
        }
    }
}
