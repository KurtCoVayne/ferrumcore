//! Bounded slice of the progressive `AdvancedPaymentScheduleTransactionProcessor`
//! (`fineract-progressive-loan/.../transactionprocessor/impl/AdvancedPaymentScheduleTransactionProcessor.java`).
//!
//! All 13 behaviors asserted by `AdvancedPaymentScheduleTransactionProcessorTest` are ported here (golden baseline
//! re-verified by running the original test — BUILD SUCCESSFUL on JDK 21):
//!
//! 1. **Charge payment** (`handleChargePayment`, src line 2368) — a single loan charge paid against the (single) due
//!    installment, fully or partially. Mirrors `payFeeChargesComponent` / `payPenaltyChargesComponent`, the
//!    `updateComponents(0, 0, fee, penalty)` tuple, and the `unprocessed = txnAmount − paidAmount` remainder.
//! 2. **Chargeback credit** (`processCreditTransaction` custom-credit-allocation branch, src line 1169, plus
//!    `calculateChargebackAllocationMap` src line 1345 and `recognizeAmountsAfterChargeback` src line 1313) — splits
//!    the chargeback amount across PRINCIPAL/INTEREST/FEE/PENALTY in the credit-allocation-rule order, capped by the
//!    original transaction's portions, then credits the matching installment (`addToCreditedPrincipal` /
//!    `addToCreditedFee` / `addToCreditedPenalty` and `addToPrincipal`), incl. crediting an **additional** (N+1)
//!    installment when no regular row is still due.
//! 3. **`findChargebackOriginalTransaction`** (src line 1002) — locating the repayment a chargeback reverses, by id,
//!    via the reverse-replay change set, or falling back to the loan's persisted transactions.
//! 4. **Disbursement after maturity** (`handleDisbursementWithEMICalculator`, src line 1646) — handing the
//!    disbursement to the [`EmiCalculator`] seam and creating the N+1 **additional** installment.
//! 5. **Payment-allocation passthrough** (`handlingPaymentAllocationForInterestBearingProgressiveLoan`, src line 2884)
//!    — paying the installment component then informing the [`EmiCalculator`] (`payPrincipal` / `payInterest`).
//!
//! Also ported: the non-interest-bearing **payment-allocation walk**, both orientations — **horizontal**
//! (`processPeriodsHorizontally` / `processAllocationsHorizontally`, src 2681/2696: allocations grouped by [`DueType`],
//! the `LoopGuard` do-while, PAST_DUE / DUE single-installment selection, IN_ADVANCE even-split — see
//! [`process_periods_horizontally`]) and **vertical** (`processPeriodsVertically`, src 3011: each allocation type
//! exhausted across its installments before the next — see [`process_periods_vertically`]). Both are verified against
//! captured Java goldens.
//!
//! **Out of scope** (the rest of the 4296-line processor + the EMI engine it drives): the top-level
//! `processLatestTransaction` transaction-type dispatcher, the
//! interest-recalculation EMI path (`processCreditTransactionWithEmiCalculator`),
//! re-age / re-amortize / capitalized-income, `adjustOriginalAllocationWithFormerChargebacks` for *prior* chargebacks,
//! overpayment allocation, and charge reprocessing. The [`EmiCalculator`] is a **seam** (trait) here — its math lives
//! in the separate progressive interest-schedule engine. The credited-* / `addToPrincipal` effects are modeled on a
//! local [`AdvancedInstallment`] view (the shared `installment.rs` is intentionally not extended).

use crate::installment::Installment;
use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
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
    /// `getFromDate()` — the period start (defaults to the due date; set via [`AdvancedInstallment::with_from_date`]).
    from_date: Date,
    /// `isAdditional()` — an N+1 chargeback installment vs. a normal schedule row.
    pub is_additional: bool,
    /// `isDownPayment()` — the down-payment row; excluded from re-amortization redistribution.
    pub is_down_payment: bool,
    /// `isReAged()` — a re-aged row; excluded from re-amortization redistribution.
    pub is_re_aged: bool,
    credited_principal: Money,
    credited_interest: Money,
    credited_fee: Money,
    credited_penalty: Money,
    /// Running principal credited via `addToPrincipal(date, amount)` (the `addToCreditedPrincipal` path also routes
    /// here). Tracked as a total because the test only inspects the (single) captured value.
    added_principal: Money,
}

impl AdvancedInstallment {
    /// Wrap a base [`Installment`] with the credited-amount view; all credited buckets start at zero and `from_date`
    /// defaults to the due date.
    #[must_use]
    pub fn new(inner: Installment, is_additional: bool) -> Self {
        let z = inner.zero();
        let from_date = inner.due_date;
        Self {
            from_date,
            credited_principal: z.clone(),
            credited_interest: z.clone(),
            credited_fee: z.clone(),
            credited_penalty: z.clone(),
            added_principal: z,
            is_additional,
            is_down_payment: false,
            is_re_aged: false,
            inner,
        }
    }

    /// Set the period start date (`getFromDate()`).
    #[must_use]
    pub fn with_from_date(mut self, from_date: Date) -> Self {
        self.from_date = from_date;
        self
    }

    /// `setFromDate` — mutate the period start in place (the re-age rebuild re-anchors surviving additional rows).
    pub fn set_from_date(&mut self, from_date: Date) {
        self.from_date = from_date;
    }

    /// `getFromDate()`.
    #[must_use]
    pub fn from_date(&self) -> Date {
        self.from_date
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

    /// Borrow the underlying schedule installment (read-only).
    #[must_use]
    pub fn installment(&self) -> &Installment {
        &self.inner
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

    /// `setCreditedPrincipal` — the model→installment sync overwrites (not accumulates) the credited principal with
    /// the engine's value.
    pub fn set_credited_principal(&mut self, amount: &Money) {
        self.credited_principal = amount.clone();
    }
    /// `setCreditedInterest`.
    pub fn set_credited_interest(&mut self, amount: &Money) {
        self.credited_interest = amount.clone();
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

// ---------------------------------------------------------------------------------------------------------------------
// findChargebackOriginalTransaction (src line 1002)
// ---------------------------------------------------------------------------------------------------------------------

/// A transaction's synthetic identity. The originals match relation targets both by object identity
/// (`Objects.equals(getToTransaction(), tx)`) and by id (`Objects.equals(getToTransaction().getId(), id)`); this key
/// models the former.
pub type TxnKey = u64;

/// The `LoanTransactionRelationTypeEnum` values the chargeback-origin lookup distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationType {
    /// `LoanTransactionRelationTypeEnum.CHARGEBACK`.
    Chargeback,
    /// Any other relation type (ignored by the lookup).
    Other,
}

/// A `LoanTransactionRelation` (subset): its type and the transaction it points *to*.
#[derive(Clone, Debug)]
pub struct TxnRelation {
    pub relation_type: RelationType,
    /// `getToTransaction().getId()` — the target's id (may be `None`).
    pub to_id: Option<i64>,
    /// The target's identity (`getToTransaction()`), for the `Objects.equals(getToTransaction(), tx)` match.
    pub to_key: TxnKey,
}

/// A `LoanTransaction` view sufficient for chargeback-origin lookup.
#[derive(Clone, Debug)]
pub struct LoanTxnView {
    /// Identity (`equals`).
    pub key: TxnKey,
    /// `getId()` — `None` during reverse-replay before the transaction is persisted.
    pub id: Option<i64>,
    /// `getLoanTransactionRelations()`.
    pub relations: Vec<TxnRelation>,
}

/// A `TransactionChangeData` (old → new) within `ChangedTransactionDetail`.
#[derive(Clone, Debug)]
pub struct TxnChange {
    /// `getOldTransaction().getId()` (the pre-replay id of the changed transaction).
    pub old_id: Option<i64>,
    /// `getNewTransaction()`.
    pub new: LoanTxnView,
}

/// `ChangedTransactionDetail.getTransactionChanges()`.
#[derive(Clone, Debug, Default)]
pub struct ChangedTransactionDetail {
    pub changes: Vec<TxnChange>,
}

/// Error mirroring the `RuntimeException("Chargeback transaction must have an original transaction")`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoOriginalTransaction;

impl core::fmt::Display for NoOriginalTransaction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Chargeback transaction must have an original transaction")
    }
}
impl std::error::Error for NoOriginalTransaction {}

/// `hasMatchingToLoanTransaction` (src line 1365/1369): a relation is a match if it is a CHARGEBACK relation whose
/// target equals `to_id` by id (`Objects.equals`, so `None == None` matches) **or** equals `chargeback_key` by
/// identity.
fn matches_chargeback(tr: &LoanTxnView, to_id: Option<i64>, chargeback_key: TxnKey) -> bool {
    tr.relations.iter().any(|r| {
        r.relation_type == RelationType::Chargeback
            && (r.to_id == to_id || r.to_key == chargeback_key)
    })
}

/// `findChargebackOriginalTransaction` (src line 1002): find the repayment a chargeback reverses.
///
/// The chargeback's id is used as the match key; during reverse-replay the chargeback has no id, so it is recovered
/// from `changed` (the `old_id` of the change whose `new` is this chargeback). The changed-transaction set is searched
/// first, then the loan's persisted transactions. Returns the original transaction's [`TxnKey`], or
/// [`NoOriginalTransaction`] if none matches.
pub fn find_chargeback_original_transaction(
    chargeback: &LoanTxnView,
    loan_transactions: &[LoanTxnView],
    changed: Option<&ChangedTransactionDetail>,
) -> Result<TxnKey, NoOriginalTransaction> {
    let mut chargeback_id = chargeback.id;
    if let Some(detail) = changed {
        if chargeback_id.is_none() {
            chargeback_id = detail
                .changes
                .iter()
                .find(|c| c.new.key == chargeback.key)
                .and_then(|c| c.old_id);
        }
        let to_id = chargeback_id;
        if let Some(from) = detail
            .changes
            .iter()
            .map(|c| &c.new)
            .find(|tr| matches_chargeback(tr, to_id, chargeback.key))
        {
            return Ok(from.key);
        }
    }
    let to_id = chargeback_id;
    loan_transactions
        .iter()
        .find(|tr| matches_chargeback(tr, to_id, chargeback.key))
        .map(|tr| tr.key)
        .ok_or(NoOriginalTransaction)
}

// ---------------------------------------------------------------------------------------------------------------------
// EMI-calculator seam + disbursement (handleDisbursementWithEMICalculator, src line 1646)
// ---------------------------------------------------------------------------------------------------------------------

/// Minimal view of `ProgressiveLoanInterestScheduleModel` — only what the ported orchestration reads/passes. The full
/// progressive interest schedule model (repayment periods, interest periods) lives in the (separate) EMI engine.
#[derive(Clone, Debug)]
pub struct ScheduleModel {
    pub maturity_date: Date,
}

/// The progressive interest engine (`EMICalculator`) the Advanced processor delegates to. Modeled as a trait *seam* so
/// the processor orchestration can be ported and verified exactly as the original unit tests do — by mocking the
/// calculator and asserting the interactions. The engine's own math (re-amortize / re-age / interest periods) is a
/// separate subsystem and is intentionally **not** implemented here.
pub trait EmiCalculator {
    /// `emiCalculator.addDisbursement(model, date, amount)`.
    fn add_disbursement(&mut self, model: &ScheduleModel, date: Date, amount: &Money);
    /// `emiCalculator.payPrincipal(model, repaymentPeriodFromDate, repaymentPeriodDueDate, payDate, amount)`.
    fn pay_principal(
        &mut self,
        model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    );
    /// `emiCalculator.payInterest(model, repaymentPeriodFromDate, repaymentPeriodDueDate, payDate, amount)`.
    fn pay_interest(
        &mut self,
        model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    );
}

/// The outcome of [`handle_disbursement_with_emi`]: the additional ("N+1") installment created when the disbursement
/// falls on/after the last scheduled installment's due date (for assertion), if any.
#[derive(Clone, Debug)]
pub struct DisbursementResult {
    pub created_additional: Option<AdvancedInstallment>,
}

/// `handleDisbursementWithEMICalculator` (src line 1646) for the tested configuration: **no** down payment and **no**
/// `installmentAmountInMultiplesOf`, so the amortizable amount equals the full disbursement. The disbursement is handed
/// to the EMI calculator, and when no non-additional installment is still due on/after the transaction date an N+1
/// **additional** installment is created carrying the amortizable principal (Java lines 1672-1689).
///
/// The downstream `recalculateRepaymentPeriodsWithEMICalculation` + `allocateOverpayment` (Java lines 1692-1694) are the
/// EMI-engine's schedule rewrite and are out of scope for this orchestration slice (the engine is a seam here).
pub fn handle_disbursement_with_emi(
    calc: &mut dyn EmiCalculator,
    model: &ScheduleModel,
    installments: &mut Vec<AdvancedInstallment>,
    scale: i64,
    mode: RoundingMode,
    txn_date: Date,
    disbursement_amount: &Money,
) -> DisbursementResult {
    let amortizable = disbursement_amount.clone();
    calc.add_disbursement(model, txn_date, &amortizable);

    // needsNPlusOneInstallment: no non-additional, non-down-payment installment due on/after the transaction date.
    let needs_n_plus_one = !installments
        .iter()
        .any(|i| !i.is_additional && i.due_date() >= txn_date);

    let mut created_additional = None;
    if needs_n_plus_one {
        let inst = Installment::new(
            scale,
            mode,
            txn_date,
            amortizable.amount().clone(),
            BigDecimal::from(0),
            BigDecimal::from(0),
            BigDecimal::from(0),
        );
        let additional = AdvancedInstallment::new(inst, true);
        created_additional = Some(additional.clone());
        installments.push(additional);
    }

    DisbursementResult { created_additional }
}

// ---------------------------------------------------------------------------------------------------------------------
// Payment allocation passthrough for interest-bearing progressive loans
// (handlingPaymentAllocationForInterestBearingProgressiveLoan, src line 2884)
// ---------------------------------------------------------------------------------------------------------------------

/// `PaymentAllocationType.getDueType()` — which installment bucket an allocation targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DueType {
    /// `DueType.PAST_DUE`.
    PastDue,
    /// `DueType.DUE`.
    Due,
    /// `DueType.IN_ADVANCE`.
    InAdvance,
}

/// A `PaymentAllocationType` (subset): which component, in which due bucket.
#[derive(Clone, Copy, Debug)]
pub struct PaymentAllocation {
    pub allocation_type: AllocationType,
    pub due_type: DueType,
}

/// `processPaymentAllocation` PAY action (src line 2013), reduced to the component bucket pay: route to the
/// installment's `pay_*` atom and return the consumed portion.
pub(crate) fn pay_component(
    inst: &mut AdvancedInstallment,
    allocation_type: AllocationType,
    amount: &Money,
) -> Money {
    let i = inst.installment_mut();
    match allocation_type {
        AllocationType::Penalty => i.pay_penalty(amount),
        AllocationType::Fee => i.pay_fee(amount),
        AllocationType::Interest => i.pay_interest(amount),
        AllocationType::Principal => i.pay_principal(amount),
    }
}

/// `handlingPaymentAllocationForInterestBearingProgressiveLoan` (src line 2884): pay the allocation's component on the
/// installment, then **inform the EMI calculator** of a principal/interest payment so it can recompute the interest
/// schedule. Returns the paid portion.
///
/// Faithful for the DUE / PAST_DUE path. **Out of scope (engine seam):** the IN_ADVANCE pay-date recalculation
/// (`calculateNewPayDateInCaseOfInAdvancePayment` + `updateRepaymentPeriodBalances`) and the post-call
/// `updateInstallmentsByRepaymentPeriods` model→installment sync (a no-op when the model has no repayment periods, as
/// in the originating unit test). The down-payment skip is represented only by the post-maturity guard.
pub fn handle_payment_allocation_interest_bearing(
    calc: &mut dyn EmiCalculator,
    model: &ScheduleModel,
    installment: &mut AdvancedInstallment,
    allocation: PaymentAllocation,
    txn_date: Date,
    unprocessed: &Money,
) -> Money {
    // Skip interest/principal model processing for periods after the loan maturity date (down-payment skip not modeled).
    if installment.due_date() > model.maturity_date {
        return pay_component(installment, allocation.allocation_type, unprocessed);
    }

    // payDate == transaction date for DUE/PAST_DUE; the IN_ADVANCE recalculation is out of scope.
    let pay_date = txn_date;
    let from_date = installment.from_date();
    let due_date = installment.due_date();

    let paid = pay_component(installment, allocation.allocation_type, unprocessed);

    match allocation.allocation_type {
        AllocationType::Principal => {
            calc.pay_principal(model, from_date, due_date, pay_date, &paid)
        }
        AllocationType::Interest => calc.pay_interest(model, from_date, due_date, pay_date, &paid),
        AllocationType::Fee | AllocationType::Penalty => {}
    }
    paid
}

/// `LoanPaymentAllocationRule.getFutureInstallmentAllocationRule()` — how an IN_ADVANCE amount spreads over future
/// installments. Consumed only by the IN_ADVANCE branch (the next bit — see `process_allocations_horizontally`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FutureInstallmentAllocationRule {
    NextInstallment,
    LastInstallment,
    Reamortization,
    NextLastInstallment,
}

/// `LoanRepaymentScheduleInstallment.isNotFullyPaidOff()` — any component still outstanding.
fn installment_not_fully_paid(inst: &AdvancedInstallment) -> bool {
    let i = inst.installment();
    i.principal_outstanding()
        .plus(&i.interest_outstanding())
        .plus(&i.fee_charges_outstanding())
        .plus(&i.penalty_charges_outstanding())
        .is_greater_than_zero()
}

/// `processPeriodsHorizontally` (src line 2681) for the **non-interest-bearing** path: group the rule's allocation
/// types by [`DueType`] in first-seen order (mirroring the Java `LinkedHashMap`), then run each group through the
/// do-while allocation loop. Returns the unprocessed amount (the overpayment). The interest-bearing path delegates
/// each leaf to the EMI seam via [`handle_payment_allocation_interest_bearing`].
#[must_use]
pub fn process_periods_horizontally(
    installments: &mut [AdvancedInstallment],
    allocations: &[PaymentAllocation],
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    amount: &Money,
) -> Money {
    let mut groups: Vec<(DueType, Vec<AllocationType>)> = Vec::new();
    for a in allocations {
        match groups.iter_mut().find(|(dt, _)| *dt == a.due_type) {
            Some(g) => g.1.push(a.allocation_type),
            None => groups.push((a.due_type, vec![a.allocation_type])),
        }
    }
    let mut unprocessed = amount.clone();
    for (due_type, types) in &groups {
        unprocessed = process_allocations_horizontally(
            installments,
            *due_type,
            types,
            future_rule,
            txn_date,
            &unprocessed,
        );
    }
    unprocessed
}

/// `processAllocationsHorizontally` (src line 2696) — the `LoopGuard` do-while (cap = installments×100) that, each
/// iteration, selects the target installment(s) for the group's `DueType` and applies each allocation type's component
/// payment, subtracting from the unprocessed amount until exhausted or every installment is fully paid.
///
/// PAST_DUE / DUE select a single installment; IN_ADVANCE spreads the unprocessed amount **evenly** across the future
/// installments chosen by `future_rule` (src 2826-2877). All three are verified against captured Java goldens (the
/// non-interest-bearing `processPaymentAllocation` path).
fn process_allocations_horizontally(
    installments: &mut [AdvancedInstallment],
    due_type: DueType,
    types: &[AllocationType],
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    amount: &Money,
) -> Money {
    let mut unprocessed = amount.clone();
    if unprocessed.is_zero() {
        return unprocessed;
    }
    let cap = installments.len().saturating_mul(100);
    let mut exit = false;
    for _ in 0..cap {
        if exit
            || !unprocessed.is_greater_than_zero()
            || !installments.iter().any(installment_not_fully_paid)
        {
            break;
        }
        // The oldest past-due / the on-due installment = the earliest in the ordered list among not-fully-paid. The
        // in-advance installments are selected per `future_rule` (all computed once per iteration, as Java does).
        let oldest_past_due = installments
            .iter()
            .position(|e| installment_not_fully_paid(e) && txn_date > e.due_date());
        let due = installments
            .iter()
            .position(|e| installment_not_fully_paid(e) && txn_date == e.due_date());
        let in_advance = select_in_advance_installments(
            installments,
            future_rule,
            txn_date,
            installment_not_fully_paid,
        );
        for &at in types {
            match due_type {
                DueType::PastDue => match oldest_past_due {
                    Some(idx) => {
                        let allocated = pay_component(&mut installments[idx], at, &unprocessed);
                        unprocessed = unprocessed.minus(&allocated);
                    }
                    None => exit = true,
                },
                DueType::Due => match due {
                    Some(idx) => {
                        let allocated = pay_component(&mut installments[idx], at, &unprocessed);
                        unprocessed = unprocessed.minus(&allocated);
                    }
                    None => exit = true,
                },
                DueType::InAdvance => {
                    let n = in_advance.len();
                    if n == 0 {
                        exit = true;
                    } else {
                        let n_bd = BigDecimal::from(n as i64);
                        // `evenPortion = unprocessed / n` (default HALF_EVEN), `balanceAdjustment` = the rounding
                        // residual that the last installment absorbs. The DOWN branch (src 2837-2845) is a guard
                        // against the split exceeding the dividend; it is unreachable for non-negative transaction
                        // amounts (for n=2 it needs evenPortion > unprocessed; for n>2 HALF_EVEN never rounds
                        // `unprocessed/n` up past `unprocessed/(n-1)`) but is transcribed faithfully.
                        let mut even = unprocessed.divided_by(&n_bd);
                        let mut adjustment = unprocessed.minus(&even.multiplied_by(&n_bd));
                        if even.plus(&adjustment).is_less_than_zero() {
                            even = unprocessed.divided_by_with_mode(&n_bd, RoundingMode::Down);
                            adjustment = unprocessed.minus(&even.multiplied_by(&n_bd));
                        }
                        for (j, &idx) in in_advance.iter().enumerate() {
                            let portion = if j == n - 1 {
                                even.plus(&adjustment)
                            } else {
                                even.clone()
                            };
                            let allocated = pay_component(&mut installments[idx], at, &portion);
                            unprocessed = unprocessed.minus(&allocated);
                        }
                    }
                }
            }
        }
    }
    unprocessed
}

/// Select the future ("in advance") installments an IN_ADVANCE amount spreads over, per `future_rule` (src 2733-2767 /
/// 3104-3142). An installment is a candidate when it is `eligible` (the horizontal walk uses `isNotFullyPaidOff`; the
/// vertical walk uses the allocation type's component-outstanding predicate) **and** the txn precedes its due date;
/// returns indices in schedule (installment-number) order.
fn select_in_advance_installments(
    installments: &[AdvancedInstallment],
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    eligible: impl Fn(&AdvancedInstallment) -> bool,
) -> Vec<usize> {
    let in_advance = |i: usize| eligible(&installments[i]) && txn_date < installments[i].due_date();
    match future_rule {
        // All not-yet-due, not-fully-paid installments.
        FutureInstallmentAllocationRule::Reamortization => {
            (0..installments.len()).filter(|&i| in_advance(i)).collect()
        }
        // The earliest such installment (min installment number).
        FutureInstallmentAllocationRule::NextInstallment => (0..installments.len())
            .find(|&i| in_advance(i))
            .into_iter()
            .collect(),
        // The latest such installment (max installment number).
        FutureInstallmentAllocationRule::LastInstallment => (0..installments.len())
            .rev()
            .find(|&i| in_advance(i))
            .into_iter()
            .collect(),
        // The "current" installment (period contains the txn date: txn on/after fromDate); else fall back to LAST.
        FutureInstallmentAllocationRule::NextLastInstallment => {
            let current: Vec<usize> = (0..installments.len())
                .filter(|&i| in_advance(i) && txn_date >= installments[i].from_date())
                .collect();
            if current.is_empty() {
                (0..installments.len())
                    .rev()
                    .find(|&i| in_advance(i))
                    .into_iter()
                    .collect()
            } else {
                current
            }
        }
    }
}

/// `getFilterPredicate` (src line 3208): whether the allocation type's component still has an outstanding balance on
/// this installment. The vertical walk selects installments by this per-component predicate (unlike the horizontal
/// walk's whole-installment `isNotFullyPaidOff`).
fn component_outstanding_gt0(inst: &AdvancedInstallment, at: AllocationType) -> bool {
    let i = inst.installment();
    match at {
        AllocationType::Principal => i.principal_outstanding().is_greater_than_zero(),
        AllocationType::Interest => i.interest_outstanding().is_greater_than_zero(),
        AllocationType::Fee => i.fee_charges_outstanding().is_greater_than_zero(),
        AllocationType::Penalty => i.penalty_charges_outstanding().is_greater_than_zero(),
    }
}

/// `processPeriodsVertically` (src line 3011) for the **non-interest-bearing** path: iterate the rule's allocation
/// types in order; for each, run a `LoopGuard` do-while that keeps paying that one component across the installments it
/// applies to — PAST_DUE = the oldest past-due installment with that component outstanding, DUE = the on-due one,
/// IN_ADVANCE = an even-split across the future installments selected by `future_rule` — until no matching installment
/// remains, the amount is exhausted, or the last step allocated nothing. (Unlike the horizontal IN_ADVANCE branch,
/// the vertical one has no `RoundingMode.DOWN` guard.) Returns the unprocessed amount (the overpayment).
#[must_use]
pub fn process_periods_vertically(
    installments: &mut [AdvancedInstallment],
    allocations: &[PaymentAllocation],
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    amount: &Money,
) -> Money {
    let mut unprocessed = amount.clone();
    let cap = installments.len().saturating_mul(100);
    for alloc in allocations {
        let at = alloc.allocation_type;
        // DO-while (LoopGuard): the body always runs once; it repeats while an installment was selected, the amount is
        // not exhausted, and the last step allocated something.
        for _ in 0..cap {
            let (installment_found, allocated_gt0) = match alloc.due_type {
                DueType::PastDue => match installments
                    .iter()
                    .position(|e| component_outstanding_gt0(e, at) && txn_date > e.due_date())
                {
                    Some(idx) => {
                        let allocated = pay_component(&mut installments[idx], at, &unprocessed);
                        let gt0 = allocated.is_greater_than_zero();
                        unprocessed = unprocessed.minus(&allocated);
                        (true, gt0)
                    }
                    None => (false, false),
                },
                DueType::Due => match installments
                    .iter()
                    .position(|e| component_outstanding_gt0(e, at) && txn_date == e.due_date())
                {
                    Some(idx) => {
                        let allocated = pay_component(&mut installments[idx], at, &unprocessed);
                        let gt0 = allocated.is_greater_than_zero();
                        unprocessed = unprocessed.minus(&allocated);
                        (true, gt0)
                    }
                    None => (false, false),
                },
                DueType::InAdvance => {
                    let list = select_in_advance_installments(
                        installments,
                        future_rule,
                        txn_date,
                        |inst| component_outstanding_gt0(inst, at),
                    );
                    let n = list.len();
                    if n == 0 {
                        (false, false)
                    } else {
                        let n_bd = BigDecimal::from(n as i64);
                        let even = unprocessed.divided_by(&n_bd);
                        let adjustment = unprocessed.minus(&even.multiplied_by(&n_bd));
                        let mut any_gt0 = false;
                        for (j, &idx) in list.iter().enumerate() {
                            let portion = if j == n - 1 {
                                even.plus(&adjustment)
                            } else {
                                even.clone()
                            };
                            let paid = pay_component(&mut installments[idx], at, &portion);
                            if paid.is_greater_than_zero() {
                                any_gt0 = true;
                            }
                            unprocessed = unprocessed.minus(&paid);
                        }
                        (true, any_gt0)
                    }
                }
            };
            if !(installment_found && unprocessed.is_greater_than_zero() && allocated_gt0) {
                break;
            }
        }
    }
    unprocessed
}

/// `LoanScheduleProcessingType` — whether the allocation walk runs horizontally (by due-type, across buckets) or
/// vertically (by bucket, across installments). `processPeriods` (src 2668) branches on `isHorizontal()`/`isVertical()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleProcessingType {
    /// `LoanScheduleProcessingType.HORIZONTAL`.
    Horizontal,
    /// `LoanScheduleProcessingType.VERTICAL`.
    Vertical,
}

/// `handleRepayment` → `processTransaction` → `processPeriods` (src 1968 / 2648 / 2668) for the non-interest-bearing
/// repayment family: run the payment-allocation walk in the loan's configured orientation and return the amount left
/// unprocessed — the overpayment that `handleOverpayment` (src 2066) would record on the overpayment holder.
#[must_use]
pub fn process_repayment(
    installments: &mut [AdvancedInstallment],
    allocations: &[PaymentAllocation],
    future_rule: FutureInstallmentAllocationRule,
    processing_type: ScheduleProcessingType,
    txn_date: Date,
    amount: &Money,
) -> Money {
    match processing_type {
        ScheduleProcessingType::Horizontal => {
            process_periods_horizontally(installments, allocations, future_rule, txn_date, amount)
        }
        ScheduleProcessingType::Vertical => {
            process_periods_vertically(installments, allocations, future_rule, txn_date, amount)
        }
    }
}

/// `handleWriteOff` (advanced src 1927 → the legacy super `handleWriteOff`,
/// `AbstractLoanRepaymentScheduleTransactionProcessor` src 764) for the non-interest-bearing / non-progressive path:
/// write off every not-fully-paid installment's outstanding principal / interest / fee / penalty (setting each
/// component's written-off amount to its outstanding), and return the aggregated written-off portions — the
/// transaction's `updateComponentsAndTotal(principal, interest, fee, penalty)` tuple.
#[must_use]
pub fn handle_write_off(installments: &mut [AdvancedInstallment]) -> CreditComponents {
    // Currency scale/mode come from any installment; the empty-schedule case (nothing to write off) is degenerate.
    let zero = installments.first().map_or_else(
        || Money::zero(2, RoundingMode::HalfEven),
        |i| i.installment().zero(),
    );
    let mut principal = zero.clone();
    let mut interest = zero.clone();
    let mut fee = zero.clone();
    let mut penalty = zero;
    for inst in installments.iter_mut() {
        if installment_not_fully_paid(inst) {
            let i = inst.installment_mut();
            principal = principal.plus(&i.write_off_principal());
            interest = interest.plus(&i.write_off_interest());
            fee = fee.plus(&i.write_off_fee());
            penalty = penalty.plus(&i.write_off_penalty());
        }
    }
    CreditComponents {
        principal,
        interest,
        fee,
        penalty,
    }
}

/// `processPaymentAllocation(.., PaymentAction.UNPAY)`'s component leaf: reverse (refund) `min(amount, paid)` out of
/// the component's paid bucket via the [`Installment`] `unpay_*` atoms, returning the deducted portion.
fn unpay_component(
    inst: &mut AdvancedInstallment,
    allocation_type: AllocationType,
    amount: &Money,
) -> Money {
    let i = inst.installment_mut();
    match allocation_type {
        AllocationType::Penalty => i.unpay_penalty(amount),
        AllocationType::Fee => i.unpay_fee(amount),
        AllocationType::Interest => i.unpay_interest(amount),
        AllocationType::Principal => i.unpay_principal(amount),
    }
}

/// The refund walk's eligibility: `getTotalPaid(currency).isGreaterThanZero()` — something must have been paid on the
/// installment for a refund to pull back.
fn installment_total_paid_gt0(inst: &AdvancedInstallment) -> bool {
    inst.installment().total_paid().is_greater_than_zero()
}

/// `refundTransactionHorizontally` (src 2418) — the refund counterpart of [`process_allocations_horizontally`]: the
/// same `LoopGuard` do-while over one due-type group, but selecting the **latest** (max installment number) paid
/// installment for PAST_DUE / DUE, spreading IN_ADVANCE across the paid future installments per `future_rule` (even
/// split, last absorbs the rounding residual, no DOWN guard), and applying the `UNPAY` leaves.
fn refund_allocations_horizontally(
    installments: &mut [AdvancedInstallment],
    due_type: DueType,
    types: &[AllocationType],
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    amount: &Money,
) -> Money {
    let mut unprocessed = amount.clone();
    let cap = installments.len().saturating_mul(100);
    let mut exit = false;
    for _ in 0..cap {
        if exit
            || !unprocessed.is_greater_than_zero()
            || !installments.iter().any(installment_total_paid_gt0)
        {
            break;
        }
        let latest_past_due = (0..installments.len()).rev().find(|&i| {
            installment_total_paid_gt0(&installments[i]) && txn_date > installments[i].due_date()
        });
        let due = (0..installments.len()).rev().find(|&i| {
            installment_total_paid_gt0(&installments[i]) && txn_date == installments[i].due_date()
        });
        let in_advance = select_in_advance_installments(
            installments,
            future_rule,
            txn_date,
            installment_total_paid_gt0,
        );
        for &at in types {
            match due_type {
                DueType::PastDue | DueType::Due => {
                    let target = if due_type == DueType::PastDue {
                        latest_past_due
                    } else {
                        due
                    };
                    match target {
                        Some(idx) => {
                            let deducted =
                                unpay_component(&mut installments[idx], at, &unprocessed);
                            unprocessed = unprocessed.minus(&deducted);
                        }
                        None => exit = true,
                    }
                }
                DueType::InAdvance => {
                    let n = in_advance.len();
                    if n == 0 {
                        exit = true;
                    } else {
                        let n_bd = BigDecimal::from(n as i64);
                        let even = unprocessed.divided_by(&n_bd);
                        let adjustment = unprocessed.minus(&even.multiplied_by(&n_bd));
                        for (j, &idx) in in_advance.iter().enumerate() {
                            let portion = if j == n - 1 {
                                even.plus(&adjustment)
                            } else {
                                even.clone()
                            };
                            let deducted = unpay_component(&mut installments[idx], at, &portion);
                            unprocessed = unprocessed.minus(&deducted);
                        }
                    }
                }
            }
        }
    }
    unprocessed
}

/// `refundTransactionVertically` (src 2512) — the refund counterpart of [`process_periods_vertically`]'s inner
/// do-while for one allocation type: keep pulling the component back from the latest paid installment (PAST_DUE / DUE)
/// or the paid future installments (IN_ADVANCE even-split) while the last step both found an installment and deducted
/// something.
fn refund_vertically(
    installments: &mut [AdvancedInstallment],
    alloc: PaymentAllocation,
    future_rule: FutureInstallmentAllocationRule,
    txn_date: Date,
    amount: &Money,
) -> Money {
    let mut unprocessed = amount.clone();
    let cap = installments.len().saturating_mul(100);
    let at = alloc.allocation_type;
    for _ in 0..cap {
        let (found, deducted_gt0) = match alloc.due_type {
            DueType::PastDue | DueType::Due => {
                let target = (0..installments.len()).rev().find(|&i| {
                    installment_total_paid_gt0(&installments[i])
                        && if alloc.due_type == DueType::PastDue {
                            txn_date > installments[i].due_date()
                        } else {
                            txn_date == installments[i].due_date()
                        }
                });
                match target {
                    Some(idx) => {
                        let deducted = unpay_component(&mut installments[idx], at, &unprocessed);
                        let gt0 = deducted.is_greater_than_zero();
                        unprocessed = unprocessed.minus(&deducted);
                        (true, gt0)
                    }
                    None => (false, false),
                }
            }
            DueType::InAdvance => {
                let list = select_in_advance_installments(
                    installments,
                    future_rule,
                    txn_date,
                    installment_total_paid_gt0,
                );
                let n = list.len();
                if n == 0 {
                    (false, false)
                } else {
                    let n_bd = BigDecimal::from(n as i64);
                    let even = unprocessed.divided_by(&n_bd);
                    let adjustment = unprocessed.minus(&even.multiplied_by(&n_bd));
                    let mut any_gt0 = false;
                    for (j, &idx) in list.iter().enumerate() {
                        let portion = if j == n - 1 {
                            even.plus(&adjustment)
                        } else {
                            even.clone()
                        };
                        let deducted = unpay_component(&mut installments[idx], at, &portion);
                        if deducted.is_greater_than_zero() {
                            any_gt0 = true;
                        }
                        unprocessed = unprocessed.minus(&deducted);
                    }
                    (true, any_gt0)
                }
            }
        };
        if !(found && unprocessed.is_greater_than_zero() && deducted_gt0) {
            break;
        }
    }
    unprocessed
}

/// `handleRefund` (src 1374) for REFUND_FOR_ACTIVE_LOAN: when the loan has no refund-specific allocation rule
/// (`is_default_rule`), the **reverse** of the default allocation order is used with a LAST_INSTALLMENT future rule
/// (src 1384-1392); the refund then runs through the configured walk orientation — horizontally by due-type group,
/// vertically by allocation type — stopping as soon as the amount is exhausted. Returns the unprocessed remainder
/// (the transaction keeps it; there is no overpayment step on the refund path).
#[must_use]
pub fn handle_refund(
    installments: &mut [AdvancedInstallment],
    allocations: &[PaymentAllocation],
    future_rule: FutureInstallmentAllocationRule,
    processing_type: ScheduleProcessingType,
    txn_date: Date,
    amount: &Money,
    is_default_rule: bool,
) -> Money {
    let (ordered, rule): (Vec<PaymentAllocation>, FutureInstallmentAllocationRule) =
        if is_default_rule {
            (
                allocations.iter().rev().copied().collect(),
                FutureInstallmentAllocationRule::LastInstallment,
            )
        } else {
            (allocations.to_vec(), future_rule)
        };
    let mut unprocessed = amount.clone();
    match processing_type {
        ScheduleProcessingType::Horizontal => {
            let mut groups: Vec<(DueType, Vec<AllocationType>)> = Vec::new();
            for a in &ordered {
                match groups.iter_mut().find(|(dt, _)| *dt == a.due_type) {
                    Some(g) => g.1.push(a.allocation_type),
                    None => groups.push((a.due_type, vec![a.allocation_type])),
                }
            }
            for (due_type, types) in &groups {
                unprocessed = refund_allocations_horizontally(
                    installments,
                    *due_type,
                    types,
                    rule,
                    txn_date,
                    &unprocessed,
                );
                if !unprocessed.is_greater_than_zero() {
                    break;
                }
            }
        }
        ScheduleProcessingType::Vertical => {
            for a in &ordered {
                unprocessed = refund_vertically(installments, *a, rule, txn_date, &unprocessed);
                if !unprocessed.is_greater_than_zero() {
                    break;
                }
            }
        }
    }
    unprocessed
}

/// The outcome of [`handle_charge_off`]: the transaction's component tuple (the outstanding sums) plus the resulting
/// charged-off state.
#[derive(Clone, Debug)]
pub struct ChargeOff {
    /// `updateComponentsAndTotal(principal, interest, fee, penalty)` — each component's total outstanding.
    pub components: CreditComponents,
    /// All components were zero → `loanTransaction.reverse()` + `loan.liftChargeOff()` fired (src 2126-2133).
    pub reversed: bool,
    /// The ctx `chargedOff` flag after the handler: set true, then cleared again when the reversal fired.
    pub charged_off: bool,
}

/// `handleChargeOff` (src 2082) for the REGULAR charge-off behaviour on a non-interest-bearing loan: stamp the
/// transaction with every installment's outstanding principal / interest / fee / penalty — **without** mutating the
/// installments (unlike [`handle_write_off`]) — and mark the context charged-off. When every component is zero the
/// transaction reverses, the charge-off is lifted, and the flag clears. (The ZERO_INTEREST / ACCELERATE_MATURITY
/// behaviours and the accrual-transaction creation are progressive-model machinery, out of scope at this seam.)
#[must_use]
pub fn handle_charge_off(installments: &[AdvancedInstallment]) -> ChargeOff {
    let zero = installments.first().map_or_else(
        || Money::zero(2, RoundingMode::HalfEven),
        |i| i.installment().zero(),
    );
    let mut principal = zero.clone();
    let mut interest = zero.clone();
    let mut fee = zero.clone();
    let mut penalty = zero;
    // Java gates the sweep on `anyMatch(isNotObligationsMet)`; when nothing is outstanding the sums stay zero either
    // way, so the observable components are identical.
    if installments.iter().any(installment_not_fully_paid) {
        for inst in installments {
            let i = inst.installment();
            principal = principal.plus(&i.principal_outstanding());
            interest = interest.plus(&i.interest_outstanding());
            fee = fee.plus(&i.fee_charges_outstanding());
            penalty = penalty.plus(&i.penalty_charges_outstanding());
        }
    }
    let all_zero = principal.is_zero() && interest.is_zero() && fee.is_zero() && penalty.is_zero();
    ChargeOff {
        components: CreditComponents {
            principal,
            interest,
            fee,
            penalty,
        },
        reversed: all_zero,
        charged_off: !all_zero,
    }
}

/// The outcome of [`handle_re_amortization`]: the transaction's principal component (the overall overdue principal
/// moved forward) and whether the transaction was reversed (`overdue == 0`, src 829-832).
#[derive(Clone, Debug)]
pub struct ReAmortization {
    /// `updateComponentsAndTotal(overdue, 0, 0, 0)` — the principal moved.
    pub principal: Money,
    /// `loanTransaction.reverse()` fired because there was nothing overdue.
    pub reversed: bool,
}

/// `Money.roundToMultiplesOf(existingVal, inMultiplesOf)` (Money.java 150): `(v / m rounded to an integer with the
/// platform rounding mode) × m`, as a raw decimal — the caller re-wraps at its currency scale.
fn round_to_multiples_of(v: &BigDecimal, multiples: i64) -> BigDecimal {
    let m = BigDecimal::from(multiples);
    let quotient = fc_money::decimal::div(v, &m, 0, RoundingMode::HalfEven);
    fc_money::decimal::mul(&quotient, &m)
}

/// `handleReAmortization` → `reamortizationOnNonInterestBearingLoan` (src 699 / 805) for the non-interest-bearing
/// path: zero out every *previous* installment's remaining principal (due on or before the transaction date) into an
/// overall overdue amount, then redistribute it evenly across the *future* installments (excluding additional /
/// down-payment / re-aged rows) — each future installment gains `overdue / n` (rounded to
/// `installment_amount_in_multiples_of` when configured), with the **latest-due** installment receiving the running
/// remainder. When nothing is overdue the transaction is reversed instead.
///
/// Divergence note: with a positive overdue and **no** eligible future installment the Java code throws
/// (`futureInstallments.stream().max(..).get()` on an empty stream); here the redistribution is skipped and the
/// overdue is only reported on the returned transaction components.
#[must_use]
pub fn handle_re_amortization(
    installments: &mut [AdvancedInstallment],
    txn_date: Date,
    installment_amount_in_multiples_of: Option<i64>,
) -> ReAmortization {
    let zero = installments.first().map_or_else(
        || Money::zero(2, RoundingMode::HalfEven),
        |i| i.installment().zero(),
    );

    // Previous installments: zero the unpaid principal into the overall overdue amount.
    let mut overdue = zero.clone();
    for inst in installments.iter_mut() {
        if inst.due_date() <= txn_date {
            let i = inst.installment_mut();
            let completed = i.principal_completed();
            overdue = overdue.plus(&i.principal().minus(&completed));
            i.update_principal(&completed);
        }
    }

    if overdue.is_zero() {
        return ReAmortization {
            principal: overdue,
            reversed: true,
        };
    }

    // Future installments (schedule rows only), the latest-due one absorbing the remainder.
    let future: Vec<usize> = (0..installments.len())
        .filter(|&i| {
            let inst = &installments[i];
            inst.due_date() > txn_date
                && !inst.is_additional
                && !inst.is_down_payment
                && !inst.is_re_aged
        })
        .collect();
    let last = future
        .iter()
        .copied()
        .max_by_key(|&i| installments[i].due_date());
    if let Some(last) = last {
        let n = BigDecimal::from(future.len() as i64);
        let mut per = overdue.divided_by(&n);
        let mut remaining = overdue.clone();
        for idx in future {
            if idx == last {
                installments[idx]
                    .installment_mut()
                    .add_to_principal(&remaining);
            } else {
                if let Some(m) = installment_amount_in_multiples_of {
                    per = installments[idx].money(&round_to_multiples_of(per.amount(), m));
                }
                installments[idx].installment_mut().add_to_principal(&per);
                remaining = remaining.minus(&per);
            }
        }
    }
    ReAmortization {
        principal: overdue,
        reversed: false,
    }
}

/// `LoanTransactionType` — the transaction kinds `processLatestTransaction` switches on (src 564-582).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvancedTransactionType {
    Disbursement,
    WriteOff,
    RefundForActiveLoan,
    Chargeback,
    CreditBalanceRefund,
    Repayment,
    MerchantIssuedRefund,
    PayoutRefund,
    GoodwillCredit,
    ChargeRefund,
    ChargeAdjustment,
    DownPayment,
    WaiveInterest,
    RecoveryRepayment,
    InterestPaymentWaiver,
    CapitalizedIncomeAdjustment,
    InterestRefund,
    ChargeOff,
    ChargePayment,
    WaiveCharges,
    ReAmortize,
    ReAge,
    CapitalizedIncome,
    ContractTermination,
}

/// The handler `processLatestTransaction` dispatches a transaction type to (src 564-582). The whole REPAYMENT family
/// (REPAYMENT / MERCHANT_ISSUED_REFUND / PAYOUT_REFUND / GOODWILL_CREDIT / CHARGE_REFUND / CHARGE_ADJUSTMENT /
/// DOWN_PAYMENT / WAIVE_INTEREST / RECOVERY_REPAYMENT / INTEREST_PAYMENT_WAIVER / CAPITALIZED_INCOME_ADJUSTMENT) routes
/// to [`TransactionRoute::Repayment`] (`handleRepayment` → the walk). WAIVE_CHARGES is logged only (a no-op).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionRoute {
    Disbursement,
    WriteOff,
    Refund,
    Chargeback,
    CreditBalanceRefund,
    Repayment,
    InterestRefund,
    ChargeOff,
    ChargePayment,
    WaiveChargesNoop,
    ReAmortize,
    ReAge,
    CapitalizedIncome,
    ContractTermination,
}

/// Route a transaction type to its handler — the faithful transcription of the `processLatestTransaction` switch
/// (src 564-582). Ported handler bodies: `Repayment` (the walk), `Chargeback` ([`process_credit_transaction`]),
/// `ChargePayment`, `Disbursement`, `WriteOff` ([`handle_write_off`]), `Refund` ([`handle_refund`]), `ChargeOff`
/// ([`handle_charge_off`]), `ReAmortize` ([`handle_re_amortization`]), and `CreditBalanceRefund` on the
/// non-interest-recalculation path (Java's super delegates straight to `processCreditTransaction`, src 799-802 —
/// the same ported function as Chargeback). Pending (progressive EMI-model machinery): `InterestRefund`, `ReAge`,
/// `CapitalizedIncome`, `ContractTermination`, and the interest-recalculation branches of the above.
#[must_use]
pub fn route_transaction(transaction_type: AdvancedTransactionType) -> TransactionRoute {
    use AdvancedTransactionType as T;
    match transaction_type {
        T::Disbursement => TransactionRoute::Disbursement,
        T::WriteOff => TransactionRoute::WriteOff,
        T::RefundForActiveLoan => TransactionRoute::Refund,
        T::Chargeback => TransactionRoute::Chargeback,
        T::CreditBalanceRefund => TransactionRoute::CreditBalanceRefund,
        T::Repayment
        | T::MerchantIssuedRefund
        | T::PayoutRefund
        | T::GoodwillCredit
        | T::ChargeRefund
        | T::ChargeAdjustment
        | T::DownPayment
        | T::WaiveInterest
        | T::RecoveryRepayment
        | T::InterestPaymentWaiver
        | T::CapitalizedIncomeAdjustment => TransactionRoute::Repayment,
        T::InterestRefund => TransactionRoute::InterestRefund,
        T::ChargeOff => TransactionRoute::ChargeOff,
        T::ChargePayment => TransactionRoute::ChargePayment,
        T::WaiveCharges => TransactionRoute::WaiveChargesNoop,
        T::ReAmortize => TransactionRoute::ReAmortize,
        T::ReAge => TransactionRoute::ReAge,
        T::CapitalizedIncome => TransactionRoute::CapitalizedIncome,
        T::ContractTermination => TransactionRoute::ContractTermination,
    }
}
