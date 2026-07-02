//! Loan-charge payment distribution — faithful port of the charge bookkeeping in
//! `AbstractLoanRepaymentScheduleTransactionProcessor` (`updateChargesPaidAmountBy` src 680,
//! `undoChargesPaidAmountBy` src 843, `findEarliestUnpaidChargeFromUnOrderedSet` src 736,
//! `findLatestPaidChargeFromUnOrderedSet` src 868) and `LoanCharge` (`updatePaidAmountBy` src 436,
//! `undoPaidOrPartiallyAmountBy` src 640, `isDueInPeriod` src 371).
//!
//! When the walk pays an installment's FEE/PENALTY component, Java distributes the paid portion across the
//! installment's `LoanCharge` entities: repeatedly pick the earliest-due unpaid charge (due-at-disbursement charges
//! excluded), pay `min(remaining, outstanding)` on it, and record a `LoanChargePaidBy(txn, charge, amount,
//! installmentNumber)`; a refund reverses the latest-paid charge first with negative records. **Scope:** regular
//! (non-installment-fee) charges; the per-installment `LoanInstallmentCharge` slices of installment-fee charges are
//! the next sub-bit (an installment-fee charge competes in the selection via its earliest unpaid slice).
//!
//! Divergence note (deny-panic totalization): Java's `undoChargesPaidAmountBy` loops `while remaining > 0` with no
//! null-break — when no paid charge remains it spins forever. The Rust loop breaks; behavior is identical whenever
//! Java terminates.

use fc_money::Money;
use time::Date;

/// The `LoanCharge` state the distribution reads and mutates (regular charges).
#[derive(Clone, Debug)]
pub struct LoanChargeView {
    /// `getAmount` — the charged amount.
    pub amount: Money,
    /// `getAmountPaid`.
    pub amount_paid: Money,
    /// `getAmountWaived`.
    pub amount_waived: Money,
    /// `getAmountOutstanding`.
    pub amount_outstanding: Money,
    /// `getDueLocalDate` — `None` never wins the earliest/latest comparisons (Java NPEs on a null due date in the
    /// comparator; such charges are pre-filtered by `isDueInPeriod` in practice).
    pub due_date: Option<Date>,
    /// `isPenaltyCharge` vs fee.
    pub is_penalty: bool,
    /// `isDueAtDisbursement` — excluded from both selections.
    pub due_at_disbursement: bool,
    /// `isPaid`.
    pub paid: bool,
    /// `isWaived`.
    pub waived: bool,
}

impl LoanChargeView {
    /// A fresh unpaid charge.
    #[must_use]
    pub fn new(amount: Money, due_date: Option<Date>, is_penalty: bool) -> Self {
        let zero = amount.minus(&amount);
        Self {
            amount_outstanding: amount.clone(),
            amount,
            amount_paid: zero.clone(),
            amount_waived: zero,
            due_date,
            is_penalty,
            due_at_disbursement: false,
            paid: false,
            waived: false,
        }
    }

    /// `LoanCharge.updatePaidAmountBy` (src 436, regular-charge branch): pay `min(increment, outstanding)`; a full
    /// payment flips `paid` (or `waived` when a waiver already covered part of the charge).
    pub fn update_paid_amount_by(&mut self, increment: &Money) -> Money {
        if increment.is_greater_than_or_equal_to(&self.amount_outstanding) {
            let portion = self.amount_outstanding.clone();
            self.amount_paid = self.amount_paid.plus(&portion);
            self.amount_outstanding = portion.minus(&portion);
            if self.amount_waived.is_greater_than_zero() {
                self.waived = true;
            } else {
                self.paid = true;
            }
            portion
        } else {
            self.amount_paid = self.amount_paid.plus(increment);
            self.amount_outstanding = self
                .amount
                .minus(&self.amount_paid)
                .minus(&self.amount_waived);
            increment.clone()
        }
    }

    /// `LoanCharge.undoPaidOrPartiallyAmountBy` (src 640, regular-charge branch): deduct `min(increment, paid)`; a
    /// full undo restores the outstanding to the charged amount and clears `paid`.
    pub fn undo_paid_amount_by(&mut self, increment: &Money) -> Money {
        if increment.is_greater_than_or_equal_to(&self.amount_paid) {
            let portion = self.amount_paid.clone();
            self.amount_paid = portion.minus(&portion);
            self.amount_outstanding = self.amount.clone();
            self.paid = false;
            portion
        } else {
            self.amount_paid = self.amount_paid.minus(increment);
            self.amount_outstanding = self
                .amount
                .minus(&self.amount_paid)
                .minus(&self.amount_waived);
            increment.clone()
        }
    }

    /// `isPaidOrPartiallyPaid` — anything collected so far.
    #[must_use]
    fn is_paid_or_partially_paid(&self) -> bool {
        self.amount_paid.is_greater_than_zero()
    }
}

/// One `LoanChargePaidBy` record: the charge index, the amount (negative for refunds), and the installment number
/// the payment was attributed to (`None` on refunds, matching Java).
#[derive(Clone, Debug)]
pub struct ChargePaidBy {
    pub charge: usize,
    pub amount: Money,
    pub installment_number: Option<u32>,
}

/// `findEarliestUnpaidChargeFromUnOrderedSet` (src 736), regular charges: the outstanding charge with the earliest
/// due date, skipping due-at-disbursement and the caller's skip set (Java removes unpayable charges from the set).
/// `only_penalty` restricts candidates to fee-vs-penalty charges (Java filters the set *before* calling — the walk's
/// FEE/PENALTY leaf passes only the matching charges); `None` considers all (a whole-installment charge payment).
fn find_earliest_unpaid_charge(
    charges: &[LoanChargeView],
    skip: &[bool],
    only_penalty: Option<bool>,
) -> Option<usize> {
    let mut earliest: Option<usize> = None;
    for (i, c) in charges.iter().enumerate() {
        if !skip[i]
            && only_penalty.is_none_or(|p| c.is_penalty == p)
            && c.amount_outstanding.is_greater_than_zero()
            && !c.due_at_disbursement
        {
            let is_earlier = match earliest {
                None => true,
                Some(e) => match (c.due_date, charges[e].due_date) {
                    (Some(a), Some(b)) => a < b,
                    _ => false,
                },
            };
            if is_earlier {
                earliest = Some(i);
            }
        }
    }
    earliest
}

/// `findLatestPaidChargeFromUnOrderedSet` (src 868), regular charges: the paid-or-partially-paid charge with the
/// latest due date, skipping due-at-disbursement. `only_penalty` mirrors [`find_earliest_unpaid_charge`].
fn find_latest_paid_charge(
    charges: &[LoanChargeView],
    only_penalty: Option<bool>,
) -> Option<usize> {
    let mut latest: Option<usize> = None;
    for (i, c) in charges.iter().enumerate() {
        if only_penalty.is_none_or(|p| c.is_penalty == p)
            && c.is_paid_or_partially_paid()
            && !c.due_at_disbursement
        {
            let is_later = match latest {
                None => true,
                Some(l) => match (c.due_date, charges[l].due_date) {
                    (Some(a), Some(b)) => a > b,
                    _ => false,
                },
            };
            if is_later {
                latest = Some(i);
            }
        }
    }
    latest
}

/// `updateChargesPaidAmountBy` (src 680): distribute a paid FEE/PENALTY component across the charges,
/// earliest-due-first, recording a positive [`ChargePaidBy`] per touched charge. A charge that accepts nothing is
/// skipped for the rest of the walk (the `chargesThatCannotBeFullyPaidByOneInstallment` set).
#[must_use]
pub fn update_charges_paid_amount_by(
    charges: &mut [LoanChargeView],
    amount: &Money,
    installment_number: Option<u32>,
    only_penalty: Option<bool>,
) -> Vec<ChargePaidBy> {
    let mut remaining = amount.clone();
    let mut records = Vec::new();
    let mut skip = vec![false; charges.len()];
    while remaining.is_greater_than_zero() {
        let Some(idx) = find_earliest_unpaid_charge(charges, &skip, only_penalty) else {
            break; // Java: "All are tranche charges"
        };
        let paid = charges[idx].update_paid_amount_by(&remaining);
        if paid.is_greater_than_zero() {
            records.push(ChargePaidBy {
                charge: idx,
                amount: paid.clone(),
                installment_number,
            });
            remaining = remaining.minus(&paid);
        } else {
            skip[idx] = true;
        }
    }
    records
}

/// `undoChargesPaidAmountBy` (src 843): reverse a refunded FEE/PENALTY component across the charges,
/// latest-paid-first, recording a **negative** [`ChargePaidBy`] (installment number `None`) per touched charge.
#[must_use]
pub fn undo_charges_paid_amount_by(
    charges: &mut [LoanChargeView],
    amount: &Money,
    only_penalty: Option<bool>,
) -> Vec<ChargePaidBy> {
    let mut remaining = amount.clone();
    let mut records = Vec::new();
    while remaining.is_greater_than_zero() {
        let Some(idx) = find_latest_paid_charge(charges, only_penalty) else {
            break; // Java spins forever here — see the module divergence note.
        };
        let deducted = charges[idx].undo_paid_amount_by(&remaining);
        if deducted.is_greater_than_zero() {
            let zero = deducted.minus(&deducted);
            records.push(ChargePaidBy {
                charge: idx,
                amount: zero.minus(&deducted),
                installment_number: None,
            });
            remaining = remaining.minus(&deducted);
        }
    }
    records
}

/// `LoanCharge.isDueInPeriod` (src 371) over the charge list: indices of the charges whose due date falls in
/// `(from, due]` (first period: `[from, due]`) — the regular-charge part of `getLoanChargesOfInstallment` (src 3001).
#[must_use]
pub fn charges_due_in_period(
    charges: &[LoanChargeView],
    from: Date,
    due: Date,
    is_first_period: bool,
) -> Vec<usize> {
    charges
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.due_date.is_some_and(|d| {
                if is_first_period {
                    from <= d && d <= due
                } else {
                    from < d && d <= due
                }
            })
        })
        .map(|(i, _)| i)
        .collect()
}
