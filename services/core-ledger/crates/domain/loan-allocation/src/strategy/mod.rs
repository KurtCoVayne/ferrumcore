//! Legacy `LoanRepaymentScheduleTransactionProcessor` strategies.
//!
//! Each strategy is the same set of installment atoms ([`crate::installment::Installment`]) called in a fixed *order*
//! for the four components, with separate handling for in-advance vs due/late payments. A strategy implements only the
//! **repayment** ordering ([`Strategy`]); the kind-aware entry points ([`allocate_on_time`], [`allocate_in_advance`],
//! [`allocate_late`], [`allocate_refund`]) layer on the common waiver / charge-payment / refund branches that every
//! original processor shares verbatim.

pub mod creocore;
pub mod due_pen_fee_int_pri;
pub mod due_pen_int_pri_fee;
pub mod early_payment;
pub mod fineract_style;
pub mod heavens_family;
pub mod interest_principal_penalty_fees_order;
pub mod principal_interest_penalty_fees_order;
pub mod rbi;

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

    /// `isTransactionInAdvanceOfInstallment` — is the payment "in advance" of the installment at `index`? Base
    /// implementation: the transaction date is before *this* installment's due date. `HeavensFamily` overrides this to
    /// compare against the *previous* installment's due date.
    fn is_in_advance(&self, index: usize, installments: &[Installment], date: Date) -> bool {
        installments.get(index).is_some_and(|i| date < i.due_date)
    }

    /// `isTransactionALateRepaymentOnInstallment` — is the payment late for the installment at `index`? Base (and only)
    /// implementation: the transaction date is after this installment's due date. No legacy strategy overrides this.
    fn is_late(&self, index: usize, installments: &[Installment], date: Date) -> bool {
        installments.get(index).is_some_and(|i| date > i.due_date)
    }

    /// Refund ordering (`handleRefundTransactionPaymentOfInstallment`). Default: [`common_refund`] (principal → interest
    /// → fee → penalty). EarlyPayment, HeavensFamily, and the Pri/Int-order strategies override this with their own
    /// distinct unpay order.
    fn refund(&self, inst: &mut Installment, amount: &Money) -> Allocation {
        common_refund(inst, amount)
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

/// One of the four payable components, used to parameterize a strategy's refund ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    Principal,
    Interest,
    Fee,
    Penalty,
}

/// Reverse a refund across the four components in `order`
/// (`handleRefundTransactionPaymentOfInstallment`): each step unpays `min(remaining, paid)` of its component while a
/// remainder is left. Every step is guarded by `remaining > 0`; the originals leave the *first* unpay unguarded, but
/// since a refund amount is always positive the first guard is always satisfied, so the result is identical.
#[must_use]
pub fn refund_in_order(
    inst: &mut Installment,
    amount: &Money,
    order: [Component; 4],
) -> Allocation {
    let mut remaining = amount.clone();
    let mut principal = inst.zero();
    let mut interest = inst.zero();
    let mut fee = inst.zero();
    let mut penalty = inst.zero();
    for component in order {
        if !remaining.is_greater_than_zero() {
            continue;
        }
        let portion = match component {
            Component::Principal => {
                principal = inst.unpay_principal(&remaining);
                principal.clone()
            }
            Component::Interest => {
                interest = inst.unpay_interest(&remaining);
                interest.clone()
            }
            Component::Fee => {
                fee = inst.unpay_fee(&remaining);
                fee.clone()
            }
            Component::Penalty => {
                penalty = inst.unpay_penalty(&remaining);
                penalty.clone()
            }
        };
        remaining = remaining.minus(&portion);
    }
    Allocation {
        principal,
        interest,
        fee,
        penalty,
        unprocessed: remaining,
    }
}

/// The refund ordering shared by the Fineract-style, Creocore, RBI, and Due* processors: unpay principal → interest →
/// fee → penalty. The default for [`Strategy::refund`].
#[must_use]
pub fn common_refund(inst: &mut Installment, amount: &Money) -> Allocation {
    refund_in_order(
        inst,
        amount,
        [
            Component::Principal,
            Component::Interest,
            Component::Fee,
            Component::Penalty,
        ],
    )
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

/// Allocate a refund (`handleRefundTransactionPaymentOfInstallment`) — delegates to the strategy's refund ordering
/// (the default is [`common_refund`]; EarlyPayment / HeavensFamily / the Pri/Int-order strategies override it).
pub fn allocate_refund(s: &dyn Strategy, inst: &mut Installment, amount: &Money) -> Allocation {
    s.refund(inst, amount)
}
