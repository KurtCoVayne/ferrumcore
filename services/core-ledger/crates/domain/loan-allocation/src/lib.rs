//! # fc-loan-allocation
//!
//! Loan payment **allocation** kernel (contract C2) — faithful port of the legacy
//! `LoanRepaymentScheduleTransactionProcessor` family
//! (`fineract-loan/.../transactionprocessor/`). Pure: no IO, no framework, no clock.
//!
//! The allocation atoms live on [`Installment`], a faithful port of the per-component arithmetic in
//! `LoanRepaymentScheduleInstallment` (`pay{Penalty,Fee,Interest,Principal}ChargesComponent`): each component tracks a
//! *charged* amount and the amounts *accounted for* (paid / waived / written off); `outstanding = charged − accounted`,
//! and a `pay_*` call consumes `min(remaining, outstanding)` into the *paid* bucket and returns the consumed portion.
//! All arithmetic is currency-scale [`Money`] (HALF_EVEN by default), exactly as the original.
//!
//! Every legacy strategy is just a fixed *order* in which those atoms are called for the four components
//! (penalty / fee / interest / principal), with separate handling for in-advance vs due/late payments. This crate
//! currently implements the [`due_pen_int_pri_fee`] strategy — proven by `tests/conformance_allocation.rs` against the
//! original's `DuePenIntPriFeeInAdvancePenIntPriFeeLoanRepaymentScheduleTransactionProcessorTest` (all 36 scenarios:
//! in-advance, on-time, late; partial payments; and due-charge-driven in-advance ordering).
//!
//! **In scope (this slice):** the repayment (non-waiver, non-charge-payment) path of one legacy strategy, at the single
//! installment level. **Out of scope (later slices):** the other legacy strategies; charge-payment / waiver / refund
//! branches; the multi-installment driver (`processLatestTransaction`); and the progressive
//! `AdvancedPaymentScheduleTransactionProcessor`. Installment side-state that does not affect the allocation outcome
//! (`obligationsMet`, advance/late running totals) is intentionally not modeled here.
#![forbid(unsafe_code)]

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_money::Money;
use time::Date;

/// One repayment-schedule installment's four payable components, each with its charged and accounted-for amounts.
///
/// Mirrors the relevant fields of `LoanRepaymentScheduleInstallment`. Constructed from the *charged* amounts (as in the
/// original test constructor); the accounted-for buckets start at zero and are advanced by the `pay_*` atoms.
#[derive(Clone, Debug)]
pub struct Installment {
    scale: i64,
    mode: RoundingMode,
    /// Period end date — distinguishes in-advance (`txn < due`) from due/late within the in-advance handler.
    pub due_date: Date,

    principal: BigDecimal,
    principal_completed: BigDecimal,
    principal_written_off: BigDecimal,

    interest_charged: BigDecimal,
    interest_paid: BigDecimal,
    interest_waived: BigDecimal,
    interest_written_off: BigDecimal,

    fee_charges_charged: BigDecimal,
    fee_charges_paid: BigDecimal,
    fee_charges_waived: BigDecimal,
    fee_charges_written_off: BigDecimal,

    penalty_charges: BigDecimal,
    penalty_charges_paid: BigDecimal,
    penalty_charges_waived: BigDecimal,
    penalty_charges_written_off: BigDecimal,
}

impl Installment {
    /// Build an installment from its charged component amounts (interest, fee, penalty, principal), all accounted-for
    /// buckets zero — matching the original `LoanRepaymentScheduleInstallment` test constructor.
    #[must_use]
    pub fn new(
        scale: i64,
        mode: RoundingMode,
        due_date: Date,
        principal: BigDecimal,
        interest: BigDecimal,
        fee_charges: BigDecimal,
        penalty_charges: BigDecimal,
    ) -> Self {
        let z = || BigDecimal::from(0);
        Self {
            scale,
            mode,
            due_date,
            principal,
            principal_completed: z(),
            principal_written_off: z(),
            interest_charged: interest,
            interest_paid: z(),
            interest_waived: z(),
            interest_written_off: z(),
            fee_charges_charged: fee_charges,
            fee_charges_paid: z(),
            fee_charges_waived: z(),
            fee_charges_written_off: z(),
            penalty_charges,
            penalty_charges_paid: z(),
            penalty_charges_waived: z(),
            penalty_charges_written_off: z(),
        }
    }

    fn money(&self, v: &BigDecimal) -> Money {
        Money::of(self.scale, self.mode, v)
    }
    /// A zero amount at this installment's currency scale/mode.
    #[must_use]
    pub fn zero(&self) -> Money {
        Money::zero(self.scale, self.mode)
    }

    /// `principal − (completed + written off)` (Java `getPrincipalOutstanding`).
    #[must_use]
    pub fn principal_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.principal_completed)
            .plus(&self.money(&self.principal_written_off));
        self.money(&self.principal).minus(&accounted)
    }
    /// `interestCharged − (paid + waived + written off)` (Java `getInterestOutstanding`).
    #[must_use]
    pub fn interest_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.interest_paid)
            .plus(&self.money(&self.interest_waived))
            .plus(&self.money(&self.interest_written_off));
        self.money(&self.interest_charged).minus(&accounted)
    }
    /// `feeChargesCharged − (paid + waived + written off)` (Java `getFeeChargesOutstanding`).
    #[must_use]
    pub fn fee_charges_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.fee_charges_paid)
            .plus(&self.money(&self.fee_charges_waived))
            .plus(&self.money(&self.fee_charges_written_off));
        self.money(&self.fee_charges_charged).minus(&accounted)
    }
    /// `penaltyCharges − (paid + waived + written off)` (Java `getPenaltyChargesOutstanding`).
    #[must_use]
    pub fn penalty_charges_outstanding(&self) -> Money {
        let accounted = self
            .money(&self.penalty_charges_paid)
            .plus(&self.money(&self.penalty_charges_waived))
            .plus(&self.money(&self.penalty_charges_written_off));
        self.money(&self.penalty_charges).minus(&accounted)
    }

    /// Consume `min(amount, penaltyOutstanding)` into the penalty *paid* bucket; return the consumed portion
    /// (Java `payPenaltyChargesComponent`).
    pub fn pay_penalty(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let due = self.penalty_charges_outstanding();
        let portion = if amount.is_greater_than_or_equal_to(&due) {
            due
        } else {
            amount.clone()
        };
        self.penalty_charges_paid = self
            .money(&self.penalty_charges_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, feeOutstanding)` into the fee *paid* bucket; return the consumed portion
    /// (Java `payFeeChargesComponent`).
    pub fn pay_fee(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let due = self.fee_charges_outstanding();
        let portion = if amount.is_greater_than_or_equal_to(&due) {
            due
        } else {
            amount.clone()
        };
        self.fee_charges_paid = self
            .money(&self.fee_charges_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, interestOutstanding)` into the interest *paid* bucket; return the consumed portion
    /// (Java `payInterestComponent`).
    pub fn pay_interest(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let due = self.interest_outstanding();
        let portion = if amount.is_greater_than_or_equal_to(&due) {
            due
        } else {
            amount.clone()
        };
        self.interest_paid = self
            .money(&self.interest_paid)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }

    /// Consume `min(amount, principalOutstanding)` into the principal *completed* bucket; return the consumed portion
    /// (Java `payPrincipalComponent`).
    pub fn pay_principal(&mut self, amount: &Money) -> Money {
        if amount.is_zero() {
            return self.zero();
        }
        let due = self.principal_outstanding();
        let portion = if amount.is_greater_than_or_equal_to(&due) {
            due
        } else {
            amount.clone()
        };
        self.principal_completed = self
            .money(&self.principal_completed)
            .plus(&portion)
            .amount()
            .clone();
        portion
    }
}

/// The outcome of allocating one transaction against one installment: the four component portions consumed (the
/// original's `loanTransaction.updateComponents(principal, interest, fee, penalty)` tuple) plus the amount left
/// unprocessed (the handler's return value).
#[derive(Clone, Debug)]
pub struct Allocation {
    pub principal: Money,
    pub interest: Money,
    pub fee: Money,
    pub penalty: Money,
    pub unprocessed: Money,
}

/// A pending loan charge, as seen by the in-advance due-charge ordering (subset of `LoanCharge` the processor reads).
#[derive(Clone, Debug)]
pub struct PendingCharge {
    /// `getEffectiveDueDate()` — `None` means "no due date" (always counts as due).
    pub effective_due_date: Option<Date>,
    /// `isPenaltyCharge()` — penalty vs fee.
    pub is_penalty: bool,
    /// `getAmount(currency)`.
    pub amount: BigDecimal,
    /// `isActive()`.
    pub is_active: bool,
    /// `isChargePending()`.
    pub is_pending: bool,
}

/// `due-penalty-interest-principal-fee-in-advance-penalty-interest-principal-fee-strategy`
/// (`DuePenIntPriFeeInAdvancePenIntPriFeeLoanRepaymentScheduleTransactionProcessor`).
///
/// On-time and late repayments pay in the order penalty → interest → principal → fee. In-advance repayments first pay
/// the *due* charges, then (in a second pass) the in-advance penalty → interest → principal → fee.
pub mod due_pen_int_pri_fee {
    use super::{Allocation, Installment, Money, PendingCharge};
    use time::Date;

    /// On-time payment: pay penalty → interest → principal → fee in full order (the repayment / non-waiver path of
    /// `handleTransactionThatIsOnTimePaymentOfInstallment`).
    pub fn on_time(inst: &mut Installment, transaction_amount: &Money) -> Allocation {
        let mut remaining = transaction_amount.clone();

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

    /// Late payment: identical ordering to on-time (the strategy delegates
    /// `handleTransactionThatIsALateRepaymentOfInstallment` to the on-time handler).
    pub fn late(inst: &mut Installment, transaction_amount: &Money) -> Allocation {
        on_time(inst, transaction_amount)
    }

    /// In-advance payment (repayment path of `handleTransactionThatIsPaymentInAdvanceOfInstallment`): pay the *due*
    /// charges first (capped at the due totals), then re-run without the due-date cap to distribute the in-advance
    /// portions. `transaction_date` decides whether interest/principal are paid in the first pass.
    pub fn in_advance(
        inst: &mut Installment,
        transaction_date: Date,
        transaction_amount: &Money,
        charges: &[PendingCharge],
    ) -> Allocation {
        let mut remaining = transaction_amount.clone();
        let mut penalty = inst.zero();
        let mut interest = inst.zero();
        let mut principal = inst.zero();
        let mut fee = inst.zero();

        // Total due penalty / fee charges (active, pending, effective due date not after the transaction date).
        let mut calc_penalty = inst.zero();
        let mut calc_fee = inst.zero();
        for c in charges.iter().filter(|c| {
            c.is_active
                && c.is_pending
                && c.effective_due_date.is_none_or(|d| d <= transaction_date)
        }) {
            let amt = inst.money(&c.amount);
            if c.is_penalty {
                calc_penalty = calc_penalty.plus(&amt);
            } else {
                calc_fee = calc_fee.plus(&amt);
            }
        }

        let mut ignore_due_date_check = false;
        let mut rerun = false;
        loop {
            // Penalty: capped at the due total on the first pass, at the full remainder thereafter.
            if !ignore_due_date_check {
                if calc_penalty.is_greater_than(&remaining) {
                    calc_penalty = remaining.clone();
                }
            } else {
                calc_penalty = remaining.clone();
            }
            let sub = inst.pay_penalty(&calc_penalty);
            remaining = remaining.minus(&sub);
            penalty = penalty.plus(&sub);

            // Interest + principal: only once the due-date cap is dropped, or for a due/late payment.
            if ignore_due_date_check || transaction_date >= inst.due_date {
                let si = inst.pay_interest(&remaining);
                remaining = remaining.minus(&si);
                interest = interest.plus(&si);

                let sp = inst.pay_principal(&remaining);
                remaining = remaining.minus(&sp);
                principal = principal.plus(&sp);
            }

            // Fee: capped at the due total on the first pass, at the full remainder thereafter.
            if !ignore_due_date_check {
                if calc_fee.is_greater_than(&remaining) {
                    calc_fee = remaining.clone();
                }
            } else {
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
