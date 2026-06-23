//! Legacy `LoanRepaymentScheduleTransactionProcessor` strategies.
//!
//! Each strategy is the same set of installment atoms ([`crate::installment::Installment`]) called in a fixed *order*
//! for the four components, with separate handling for in-advance vs due/late payments. A strategy implements only the
//! **repayment** ordering ([`Strategy`]); the kind-aware entry points ([`allocate_on_time`], [`allocate_in_advance`],
//! [`allocate_late`], [`allocate_refund`]) layer on the common waiver / charge-payment / refund branches that every
//! original processor shares verbatim.

pub mod due_pen_int_pri_fee;

use crate::installment::Installment;
use crate::model::{Allocation, PendingCharge, Txn, TxnKind};
use fc_money::Money;
use time::Date;

/// A legacy payment-allocation strategy: the *repayment* component ordering for each payment timing.
pub trait Strategy {
    /// The original `STRATEGY_CODE`.
    fn code(&self) -> &'static str;
    /// The original `STRATEGY_NAME`.
    fn name(&self) -> &'static str;

    /// Repayment ordering for an on-time payment (the `else` branch of
    /// `handleTransactionThatIsOnTimePaymentOfInstallment`).
    fn on_time_repayment(&self, inst: &mut Installment, date: Date, amount: &Money) -> Allocation;

    /// Repayment ordering for an in-advance payment (the `else` branch of
    /// `handleTransactionThatIsPaymentInAdvanceOfInstallment`).
    fn in_advance_repayment(
        &self,
        inst: &mut Installment,
        date: Date,
        amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation;

    /// Repayment ordering for a late payment. Defaults to the on-time ordering (as the Fineract-style and Due*
    /// strategies do); strategies with a distinct late path override this.
    fn late_repayment(
        &self,
        inst: &mut Installment,
        date: Date,
        amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation {
        let _ = charges;
        self.on_time_repayment(inst, date, amount)
    }
}

/// The waiver / charge-payment branches shared verbatim by every legacy processor's timing handlers. Returns `None`
/// for an ordinary repayment (so the caller runs the strategy ordering).
fn nonrepayment(inst: &mut Installment, txn: &Txn) -> Option<Allocation> {
    let zero = inst.zero();
    match &txn.kind {
        TxnKind::Repayment => None,
        TxnKind::ChargesWaiver {
            penalty_portion,
            fee_portion,
        } => {
            let mut remaining = txn.amount.clone();
            let pen_amt = inst.money(penalty_portion);
            let penalty = inst.waive_penalty(&pen_amt);
            remaining = remaining.minus(&penalty);
            let fee_amt = inst.money(fee_portion);
            let fee = inst.waive_fee(&fee_amt);
            remaining = remaining.minus(&fee);
            Some(Allocation {
                principal: zero.clone(),
                interest: zero,
                fee,
                penalty,
                unprocessed: remaining,
            })
        }
        TxnKind::InterestWaiver => {
            let mut remaining = txn.amount.clone();
            let interest = inst.waive_interest(&remaining);
            remaining = remaining.minus(&interest);
            Some(Allocation {
                principal: zero.clone(),
                interest,
                fee: zero.clone(),
                penalty: zero,
                unprocessed: remaining,
            })
        }
        TxnKind::ChargePayment { is_penalty } => {
            let mut remaining = txn.amount.clone();
            let (mut penalty, mut fee) = (zero.clone(), zero.clone());
            if *is_penalty {
                penalty = inst.pay_penalty(&remaining);
                remaining = remaining.minus(&penalty);
            } else {
                fee = inst.pay_fee(&remaining);
                remaining = remaining.minus(&fee);
            }
            Some(Allocation {
                principal: zero.clone(),
                interest: zero,
                fee,
                penalty,
                unprocessed: remaining,
            })
        }
    }
}

/// Total *due* penalty and fee charges (active, pending, effective due date not after `date`) — the in-advance
/// handlers' `calculatedPenaltyCharge` / `calculatedFeeCharge`. Sort order is irrelevant (the originals only sum).
#[must_use]
pub fn due_charge_totals(
    inst: &Installment,
    date: Date,
    charges: &[PendingCharge],
) -> (Money, Money) {
    let mut penalty = inst.zero();
    let mut fee = inst.zero();
    for c in charges
        .iter()
        .filter(|c| c.is_active && c.is_pending && c.effective_due_date.is_none_or(|d| d <= date))
    {
        let amt = inst.money(&c.amount);
        if c.is_penalty {
            penalty = penalty.plus(&amt);
        } else {
            fee = fee.plus(&amt);
        }
    }
    (penalty, fee)
}

/// The refund ordering shared by the Fineract-style and Due* processors
/// (`handleRefundTransactionPaymentOfInstallment`): unpay principal → interest → fee → penalty, guarded so each step
/// only runs while a remainder is left (principal always runs first).
#[must_use]
pub fn common_refund(inst: &mut Installment, amount: &Money) -> Allocation {
    let mut remaining = amount.clone();
    let principal = inst.unpay_principal(&remaining);
    remaining = remaining.minus(&principal);

    let mut interest = inst.zero();
    let mut fee = inst.zero();
    let mut penalty = inst.zero();
    if remaining.is_greater_than_zero() {
        interest = inst.unpay_interest(&remaining);
        remaining = remaining.minus(&interest);
    }
    if remaining.is_greater_than_zero() {
        fee = inst.unpay_fee(&remaining);
        remaining = remaining.minus(&fee);
    }
    if remaining.is_greater_than_zero() {
        penalty = inst.unpay_penalty(&remaining);
        remaining = remaining.minus(&penalty);
    }
    Allocation {
        principal,
        interest,
        fee,
        penalty,
        unprocessed: remaining,
    }
}

/// Allocate an on-time transaction: common waiver / charge-payment branch, else the strategy's on-time ordering.
pub fn allocate_on_time(s: &dyn Strategy, inst: &mut Installment, txn: &Txn) -> Allocation {
    match nonrepayment(inst, txn) {
        Some(a) => a,
        None => s.on_time_repayment(inst, txn.date, &txn.amount),
    }
}

/// Allocate an in-advance transaction: common branch, else the strategy's in-advance ordering (with due charges).
pub fn allocate_in_advance(
    s: &dyn Strategy,
    inst: &mut Installment,
    txn: &Txn,
    charges: &[PendingCharge],
) -> Allocation {
    match nonrepayment(inst, txn) {
        Some(a) => a,
        None => s.in_advance_repayment(inst, txn.date, &txn.amount, charges),
    }
}

/// Allocate a late transaction: common branch, else the strategy's late ordering.
pub fn allocate_late(
    s: &dyn Strategy,
    inst: &mut Installment,
    txn: &Txn,
    charges: &[PendingCharge],
) -> Allocation {
    match nonrepayment(inst, txn) {
        Some(a) => a,
        None => s.late_repayment(inst, txn.date, &txn.amount, charges),
    }
}

/// Allocate a refund (`handleRefundTransactionPaymentOfInstallment`) — currently the common ordering for all strategies.
pub fn allocate_refund(_s: &dyn Strategy, inst: &mut Installment, amount: &Money) -> Allocation {
    common_refund(inst, amount)
}
