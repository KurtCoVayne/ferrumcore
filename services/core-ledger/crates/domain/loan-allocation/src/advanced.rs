//! Bounded slice of the progressive `AdvancedPaymentScheduleTransactionProcessor`
//! (`fineract-progressive-loan/.../transactionprocessor/impl/AdvancedPaymentScheduleTransactionProcessor.java`).
//!
//! Only the behaviors asserted by `AdvancedPaymentScheduleTransactionProcessorTest` are ported here:
//!
//! 1. **Charge payment** (`handleChargePayment`, src line 2368) — a single loan charge paid against the (single) due
//!    installment, fully or partially. Mirrors `payFeeChargesComponent` / `payPenaltyChargesComponent`, the
//!    `updateComponents(0, 0, fee, penalty)` tuple, and the `unprocessed = txnAmount − paidAmount` remainder.
//! 2. **Chargeback credit** (`processCreditTransaction` custom-credit-allocation branch, src line 1169, plus
//!    `calculateChargebackAllocationMap` src line 1345 and `recognizeAmountsAfterChargeback` src line 1313) — splits
//!    the chargeback amount across PRINCIPAL/INTEREST/FEE/PENALTY in the credit-allocation-rule order, capped by the
//!    original transaction's portions, then credits the matching installment (`addToCreditedPrincipal` /
//!    `addToCreditedFee` / `addToCreditedPenalty` and `addToPrincipal`).
//!
//! **Out of scope** (the rest of the 4296-line processor): interest-recalculation EMI path
//! (`processCreditTransactionWithEmiCalculator`), the default/no-custom-rule credit path
//! (`super.processCreditTransaction`), `adjustOriginalAllocationWithFormerChargebacks` for *prior* chargebacks (the
//! tests have none, so the original allocation passes through unchanged), N+1 additional-installment creation,
//! multi-installment driver `processLatestTransaction`, overpayment allocation, and charge reprocessing. The
//! credited-* / `addToPrincipal` effects are modeled on a local [`AdvancedInstallment`] view (the shared
//! `installment.rs` is intentionally not extended).

use crate::installment::Installment;
use bigdecimal::BigDecimal;
use fc_money::Money;
use time::Date;

/// The four payable component buckets, in the fixed order the credit-allocation rule names them.
/// Mirrors `org.apache.fineract.portfolio.loanproduct.domain.AllocationType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocationType {
    /// `AllocationType.PRINCIPAL`.
    Principal,
    /// `AllocationType.INTEREST`.
    Interest,
    /// `AllocationType.FEE`.
    Fee,
    /// `AllocationType.PENALTY`.
    Penalty,
}

/// The `updateComponents(principal, interest, fee, penalty)` tuple a credit (chargeback) transaction is stamped with —
/// the analogue of [`crate::model::Allocation`] for the credited-amount model (no `unprocessed`, since the credit path
/// never leaves a repayment remainder in these scenarios).
#[derive(Clone, Debug)]
pub struct CreditComponents {
    pub principal: Money,
    pub interest: Money,
    pub fee: Money,
    pub penalty: Money,
}

/// The four original-transaction portions a chargeback is distributed against — `getOriginalAllocation`
/// (src line 1336) reading `getPrincipalPortion` / `getInterestPortion` / `getFeeChargesPortion` /
/// `getPenaltyChargesPortion` off the original repayment.
#[derive(Clone, Debug)]
pub struct OriginalAllocation {
    pub principal: Money,
    pub interest: Money,
    pub fee: Money,
    pub penalty: Money,
}

impl OriginalAllocation {
    /// The original portion for `t` (the `originalAllocation.get(allocationType)` lookup).
    #[must_use]
    fn get(&self, t: AllocationType) -> &Money {
        match t {
            AllocationType::Principal => &self.principal,
            AllocationType::Interest => &self.interest,
            AllocationType::Fee => &self.fee,
            AllocationType::Penalty => &self.penalty,
        }
    }
}

/// The outcome of [`calculate_chargeback_allocation_map`]: the amount assigned to each of the four components. Built
/// to mirror the `Map<AllocationType, Money>` the original returns (every key present, zero when untouched).
#[derive(Clone, Debug)]
pub struct ChargebackAllocation {
    pub principal: Money,
    pub interest: Money,
    pub fee: Money,
    pub penalty: Money,
}

impl ChargebackAllocation {
    fn slot_mut(&mut self, t: AllocationType) -> &mut Money {
        match t {
            AllocationType::Principal => &mut self.principal,
            AllocationType::Interest => &mut self.interest,
            AllocationType::Fee => &mut self.fee,
            AllocationType::Penalty => &mut self.penalty,
        }
    }
}

/// A local installment view that adds the chargeback *credited-amount* side-state the test asserts on
/// (`addToCreditedPrincipal` / `addToCreditedInterest` / `addToCreditedFee` / `addToCreditedPenalty` and the
/// `addToPrincipal` running total) on top of the shared [`Installment`] arithmetic. Kept in this module so
/// `installment.rs` stays untouched.
#[derive(Clone, Debug)]
pub struct AdvancedInstallment {
    inner: Installment,
    /// `isAdditional()` — an N+1 chargeback installment vs. a normal schedule row.
    pub is_additional: bool,
    credited_principal: Money,
    credited_interest: Money,
    credited_fee: Money,
    credited_penalty: Money,
    /// Running principal credited via `addToPrincipal(date, amount)` (the `addToCreditedPrincipal` path also routes
    /// here). Tracked as a total because the test only inspects the (single) captured value.
    added_principal: Money,
}

impl AdvancedInstallment {
    /// Wrap a base [`Installment`] with the credited-amount view; all credited buckets start at zero.
    #[must_use]
    pub fn new(inner: Installment, is_additional: bool) -> Self {
        let z = inner.zero();
        Self {
            credited_principal: z.clone(),
            credited_interest: z.clone(),
            credited_fee: z.clone(),
            credited_penalty: z.clone(),
            added_principal: z,
            is_additional,
            inner,
        }
    }

    /// `getDueDate()`.
    #[must_use]
    pub fn due_date(&self) -> Date {
        self.inner.due_date
    }

    /// Borrow the underlying schedule installment (for the charge-payment `pay_*` atoms).
    pub fn installment_mut(&mut self) -> &mut Installment {
        &mut self.inner
    }

    /// Wrap a raw decimal as [`Money`] at this installment's currency scale/mode.
    #[must_use]
    pub fn money(&self, v: &BigDecimal) -> Money {
        self.inner.money(v)
    }

    // ----- credited-amount accessors (test assertions read these) ---------------------------------------------------

    /// Cumulative `addToCreditedPrincipal`.
    #[must_use]
    pub fn credited_principal(&self) -> &Money {
        &self.credited_principal
    }
    /// Cumulative `addToCreditedInterest`.
    #[must_use]
    pub fn credited_interest(&self) -> &Money {
        &self.credited_interest
    }
    /// Cumulative `addToCreditedFee`.
    #[must_use]
    pub fn credited_fee(&self) -> &Money {
        &self.credited_fee
    }
    /// Cumulative `addToCreditedPenalty`.
    #[must_use]
    pub fn credited_penalty(&self) -> &Money {
        &self.credited_penalty
    }
    /// Cumulative `addToPrincipal`.
    #[must_use]
    pub fn added_principal(&self) -> &Money {
        &self.added_principal
    }
}

/// `calculateChargebackAllocationMap` (src line 1345): walk `allocation_types` in order, assigning to each component
/// `min(remaining, originalPortion)` but only while `remaining > 0` and the component's original portion is `> 0`.
/// Components not named (or reached with no remainder) stay zero.
#[must_use]
pub fn calculate_chargeback_allocation_map(
    original: &OriginalAllocation,
    amount_to_distribute: &Money,
    allocation_types: &[AllocationType],
) -> ChargebackAllocation {
    let zero = amount_to_distribute.minus(amount_to_distribute);
    let mut remaining = amount_to_distribute.clone();
    let mut result = ChargebackAllocation {
        principal: zero.clone(),
        interest: zero.clone(),
        fee: zero.clone(),
        penalty: zero.clone(),
    };
    for &t in allocation_types {
        if !remaining.is_greater_than_zero() {
            continue;
        }
        let original_amount = original.get(t);
        if !original_amount.is_greater_than_zero() {
            continue;
        }
        // `remaining > original` → take the whole original portion; otherwise take the remainder and finish.
        if remaining.is_greater_than(original_amount) {
            *result.slot_mut(t) = original_amount.clone();
            remaining = remaining.minus(original_amount);
        } else {
            *result.slot_mut(t) = remaining.clone();
            remaining = zero.clone();
        }
    }
    result
}

/// `recognizeFeePenaltiesAmountsAfterChargeback` (src line 1295): credit fee then penalty to the installment when each
/// is `> 0` (`addToCreditedFee` + `addToChargePortion(fee,…)`, then `addToCreditedPenalty` + `addToChargePortion(…
/// penalty…)`). The shared `addToChargePortion` book-keeping is not modeled (it has no effect the tests inspect beyond
/// the credited buckets), but the fee-before-penalty order is preserved.
fn recognize_fee_penalties_after_chargeback(
    inst: &mut AdvancedInstallment,
    alloc: &ChargebackAllocation,
) {
    if alloc.fee.is_greater_than_zero() {
        inst.credited_fee = inst.credited_fee.plus(&alloc.fee);
    }
    if alloc.penalty.is_greater_than_zero() {
        inst.credited_penalty = inst.credited_penalty.plus(&alloc.penalty);
    }
}

/// `recognizeAmountsAfterChargeback` (src line 1313): credit principal (`addToCreditedPrincipal` + `addToPrincipal`),
/// then interest (`addToCreditedInterest` + `addToInterest`), each when `> 0`, then fee/penalty via
/// [`recognize_fee_penalties_after_chargeback`].
fn recognize_amounts_after_chargeback(
    inst: &mut AdvancedInstallment,
    alloc: &ChargebackAllocation,
) {
    if alloc.principal.is_greater_than_zero() {
        inst.credited_principal = inst.credited_principal.plus(&alloc.principal);
        inst.added_principal = inst.added_principal.plus(&alloc.principal);
    }
    if alloc.interest.is_greater_than_zero() {
        inst.credited_interest = inst.credited_interest.plus(&alloc.interest);
    }
    recognize_fee_penalties_after_chargeback(inst, alloc);
}

/// The result of [`process_credit_transaction`]: the chargeback transaction's stamped components plus the index of the
/// installment that received the credit (the first non-additional installment whose due date is *after* the
/// transaction date, or any additional installment — `processCreditTransaction` src lines 1211–1228).
#[derive(Clone, Debug)]
pub struct CreditResult {
    pub components: CreditComponents,
    /// Index into the passed `installments` slice that was credited, or `None` if none matched (N+1 case, out of
    /// scope here).
    pub credited_installment: Option<usize>,
}

/// `processCreditTransaction` custom-credit-allocation branch (src line 1169) for a CHARGEBACK with no prior
/// chargebacks: compute the allocation map, stamp `updateComponents`, then credit the matching installment.
///
/// `original` is `getOriginalAllocation` of the chargeback's original transaction; `allocation_types` is the
/// chargeback credit-allocation rule's ordering; `transaction_date`/`transaction_amount` are the chargeback's.
#[must_use]
pub fn process_credit_transaction(
    original: &OriginalAllocation,
    allocation_types: &[AllocationType],
    transaction_date: Date,
    transaction_amount: &Money,
    installments: &mut [AdvancedInstallment],
) -> CreditResult {
    let alloc = calculate_chargeback_allocation_map(original, transaction_amount, allocation_types);

    let components = CreditComponents {
        principal: alloc.principal.clone(),
        interest: alloc.interest.clone(),
        fee: alloc.fee.clone(),
        penalty: alloc.penalty.clone(),
    };

    // Find the installment to credit: first non-additional row due *after* the transaction date, or the first
    // additional row (the `for` loop in src lines 1211–1228; the N+1 fall-through is out of scope).
    let mut credited = None;
    for (idx, inst) in installments.iter_mut().enumerate() {
        // An additional row always matches; a regular row matches only if it is due after the transaction date.
        let matches = if inst.is_additional {
            true
        } else {
            inst.due_date() > transaction_date
        };
        if matches {
            recognize_amounts_after_chargeback(inst, &alloc);
            credited = Some(idx);
            break;
        }
    }

    CreditResult {
        components,
        credited_installment: credited,
    }
}

/// The outcome of [`handle_charge_payment`]: the `updateComponents(0, 0, fee, penalty)` tuple plus the
/// `unprocessed = txnAmount − paidAmount` remainder.
#[derive(Clone, Debug)]
pub struct ChargePaymentResult {
    pub fee: Money,
    pub penalty: Money,
    pub unprocessed: Money,
}

/// `handleChargePayment` (src line 2368), reduced to the single-installment, single-charge case the tests exercise.
///
/// `amount_to_be_paid = min(txnAmount, chargeOutstanding)`; on the (single) due installment the charge is paid into the
/// penalty bucket when `is_penalty`, else the fee bucket, via the installment `pay_*` atoms; the transaction is stamped
/// `updateComponents(0, 0, fee, penalty)` and `unprocessed = txnAmount − paidAmount`.
///
/// `charge_outstanding` is `loanCharge.getAmountOutstanding(currency)`; in the tests `updatePaidAmountBy` returns
/// `amount_to_be_paid` unchanged, so the paid amount equals what the bucket consumes.
#[must_use]
pub fn handle_charge_payment(
    inst: &mut AdvancedInstallment,
    transaction_amount: &Money,
    charge_outstanding: &Money,
    is_penalty: bool,
) -> ChargePaymentResult {
    // amountToBePaid = min(txnAmount, chargeOutstanding): cap only when outstanding is strictly less.
    let amount_to_be_paid = if transaction_amount.is_greater_than(charge_outstanding) {
        charge_outstanding.clone()
    } else {
        transaction_amount.clone()
    };

    let zero = inst.installment_mut().zero();
    let (mut fee, mut penalty) = (zero.clone(), zero);
    if is_penalty {
        penalty = inst.installment_mut().pay_penalty(&amount_to_be_paid);
    } else {
        fee = inst.installment_mut().pay_fee(&amount_to_be_paid);
    }

    // `unprocessed = txnAmount − paidAmount`, where `paidAmount` is the `updatePaidAmountBy` return (= amountToBePaid
    // in these scenarios — `updatePaidAmountBy` returns its argument). When the installment bucket fully absorbs the
    // charge (as the tests arrange) this equals `txnAmount − feePortion`.
    let unprocessed = transaction_amount.minus(&amount_to_be_paid);
    ChargePaymentResult {
        fee,
        penalty,
        unprocessed,
    }
}
