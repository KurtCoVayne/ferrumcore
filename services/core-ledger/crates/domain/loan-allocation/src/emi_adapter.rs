//! Wires the real `fc-loan-emi` progressive interest engine behind the [`EmiCalculator`] seam that the Advanced
//! payment-allocation processor delegates to.
//!
//! The Advanced processor was ported against a trait *seam* ([`crate::advanced::EmiCalculator`]) so its orchestration
//! could be verified by mocking the calculator. This adapter closes that seam with the actual engine: it wraps a
//! [`fc_loan_emi::recalc::Model`] (the authoritative repayment / interest-period schedule) and forwards the three seam
//! operations to it. The processor's minimal [`ScheduleModel`] view (just the maturity date) is ignored — the wrapped
//! engine carries the real schedule state, exactly as the original `ProgressiveLoanInterestScheduleModel` does.

use crate::advanced::{
    AdvancedInstallment, AllocationType, DueType, EmiCalculator, PaymentAllocation, ScheduleModel,
    pay_component,
};
use crate::installment::Installment;
use bigdecimal::BigDecimal;
use fc_loan_emi::recalc::Model;
use fc_money::Money;
use time::Date;

/// `RecalculationFrequencyType` (the loan's interest-recalculation rest frequency), restricted to the two variants the
/// progressive processor accepts — Java throws `IllegalStateException` for WEEKLY / MONTHLY / INVALID
/// (`shouldRecalculateTillInstallmentDueDate`, src 2977).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestFrequencyType {
    /// `RecalculationFrequencyType.DAILY`.
    Daily,
    /// `RecalculationFrequencyType.SAME_AS_REPAYMENT_PERIOD`.
    SameAsRepaymentPeriod,
}

/// `shouldRecalculateTillInstallmentDueDate` (src 2977): whether an in-advance payment recalculates interest to the
/// installment's due date (rest-frequency date) instead of the transaction date. DAILY defers to the rest-frequency
/// strategy only on a prepay attempt; SAME_AS_REPAYMENT_PERIOD follows the strategy directly.
#[must_use]
pub fn should_recalculate_till_installment_due_date(
    rest_frequency: RestFrequencyType,
    calculate_till_rest_frequency_enabled: bool,
    is_prepay_attempt: bool,
) -> bool {
    match rest_frequency {
        RestFrequencyType::Daily => is_prepay_attempt && calculate_till_rest_frequency_enabled,
        RestFrequencyType::SameAsRepaymentPeriod => calculate_till_rest_frequency_enabled,
    }
}

/// `calculateNewPayDateInCaseOfInAdvancePayment` (src 2967): when the configuration says to recalculate till the
/// installment due date AND the transaction falls inside the installment's period (`from < txn <= due`, the
/// non-first-period `isInPeriod`), the pay date snaps to the installment's due date; otherwise it stays the
/// transaction date.
#[must_use]
pub fn calculate_new_pay_date_in_case_of_in_advance_payment(
    txn_date: Date,
    installment_from: Date,
    installment_due: Date,
    recalculate_till_due_date: bool,
) -> Date {
    if recalculate_till_due_date && installment_from < txn_date && txn_date <= installment_due {
        installment_due
    } else {
        txn_date
    }
}

/// `calculateIsPrepayAttempt` (src 2949): the transaction amount covers the loan's entire outstanding
/// (principal + interest) recalculated to the related installment's due date.
#[must_use]
pub fn calculate_is_prepay_attempt(
    model: &Model,
    installment_due: Date,
    txn_amount: &Money,
) -> bool {
    let (principal, interest) = model.get_outstanding_amounts_till_date(installment_due);
    txn_amount.is_greater_than_or_equal_to(&principal.plus(&interest))
}

/// `updateInstallmentsByRepaymentPeriods` (src 2918) — the model→installment sync: for every engine repayment period,
/// find the installment with the same from/due dates (skipping down-payment rows) and overwrite its charged principal
/// / interest and credited amounts with the engine's recalculated values. (The Java `updateObligationsMet` refresh is
/// a derived flag not modeled on [`AdvancedInstallment`].)
pub fn update_installments_by_repayment_periods(
    model: &Model,
    installments: &mut [AdvancedInstallment],
) {
    for i in 0..model.periods.len() {
        let (from, due) = (model.periods[i].from, model.periods[i].due);
        if let Some(inst) = installments
            .iter_mut()
            .find(|ri| ri.from_date() == from && ri.due_date() == due && !ri.is_down_payment)
        {
            inst.installment_mut()
                .update_principal(&model.period_due_principal(i));
            inst.installment_mut()
                .update_interest_charged(&model.period_due_interest(i));
            inst.set_credited_interest(&model.period_credited_interest(i));
            inst.set_credited_principal(&model.period_credited_principal(i));
        }
    }
}

/// `updateRepaymentPeriodBalances` (src 2933): before an IN_ADVANCE allocation pays an installment, refresh its
/// charged amount from the engine's `getDueAmounts(from, due, payDate)` — IN_ADVANCE_PRINCIPAL overwrites the charged
/// principal, IN_ADVANCE_INTEREST the charged interest (fee/penalty untouched). Returns `true` when both due amounts
/// are zero — the installment joins the ctx's skip list (`getSkipRepaymentScheduleInstallments`).
#[must_use]
pub fn update_repayment_period_balances(
    model: &Model,
    installment: &mut AdvancedInstallment,
    allocation_type: AllocationType,
    pay_date: Date,
) -> bool {
    let Some(details) =
        model.get_due_amounts(installment.from_date(), installment.due_date(), pay_date)
    else {
        return false;
    };
    let skip = details.due_interest.is_zero() && details.due_principal.is_zero();
    match allocation_type {
        AllocationType::Interest => installment
            .installment_mut()
            .update_interest_charged(&details.due_interest),
        AllocationType::Principal => installment
            .installment_mut()
            .update_principal(&details.due_principal),
        AllocationType::Fee | AllocationType::Penalty => {}
    }
    skip
}

/// The outcome of [`handle_payment_allocation_progressive`]: the paid portion plus whether the installment joined the
/// ctx skip list (down-payment / after-maturity, or an IN_ADVANCE refresh finding nothing due).
#[derive(Clone, Debug)]
pub struct ProgressivePaid {
    pub paid: Money,
    pub skip: bool,
}

/// `handlingPaymentAllocationForInterestBearingProgressiveLoan` (src 2884) against the **real engine model** — the
/// full-fidelity version of the seam-level `handle_payment_allocation_interest_bearing`:
///
/// 1. down-payment rows and installments due after the model's maturity skip the engine entirely (plain component
///    pay + skip list, src 2892-2898);
/// 2. IN_ADVANCE allocations first recalculate the pay date ([`calculate_new_pay_date_in_case_of_in_advance_payment`])
///    and refresh the installment's charge from the engine ([`update_repayment_period_balances`], src 2900-2903);
/// 3. the component is paid, and PRINCIPAL / INTEREST portions are recorded on the engine
///    (`payPrincipal` / `payInterest` with the recalculated pay date) followed by the model→installment sync
///    ([`update_installments_by_repayment_periods`], src 2905-2914).
#[must_use]
pub fn handle_payment_allocation_progressive(
    model: &mut Model,
    installments: &mut [AdvancedInstallment],
    idx: usize,
    allocation: PaymentAllocation,
    txn_date: Date,
    unprocessed: &Money,
    recalculate_till_due_date: bool,
) -> ProgressivePaid {
    let (from, due, is_down_payment) = {
        let inst = &installments[idx];
        (inst.from_date(), inst.due_date(), inst.is_down_payment)
    };
    let maturity = model.periods.last().map(|p| p.due);

    // Skip engine processing for down-payment periods or periods after the loan maturity date.
    if is_down_payment || maturity.is_some_and(|m| due > m) {
        return ProgressivePaid {
            paid: pay_component(
                &mut installments[idx],
                allocation.allocation_type,
                unprocessed,
            ),
            skip: true,
        };
    }

    let mut pay_date = txn_date;
    let mut skip = false;
    if allocation.due_type == DueType::InAdvance {
        pay_date = calculate_new_pay_date_in_case_of_in_advance_payment(
            txn_date,
            from,
            due,
            recalculate_till_due_date,
        );
        skip = update_repayment_period_balances(
            model,
            &mut installments[idx],
            allocation.allocation_type,
            pay_date,
        );
    }

    let paid = pay_component(
        &mut installments[idx],
        allocation.allocation_type,
        unprocessed,
    );

    match allocation.allocation_type {
        AllocationType::Principal => {
            model.pay_principal(from, due, pay_date, &paid);
            update_installments_by_repayment_periods(model, installments);
        }
        AllocationType::Interest => {
            model.pay_interest(from, due, pay_date, &paid);
            update_installments_by_repayment_periods(model, installments);
        }
        AllocationType::Fee | AllocationType::Penalty => {}
    }
    ProgressivePaid { paid, skip }
}

/// The outcome of [`handle_re_age_progressive`]: the transaction's principal / interest components (the precalculated
/// re-age outstanding) and whether the zero-amount reversal fired.
#[derive(Clone, Debug)]
pub struct ReAgeOutcome {
    pub principal: Money,
    pub interest: Money,
    pub reversed: bool,
}

/// `handleReAgeEqualAmortizationEMICalculator` (src 3806) for the fee-/penalty-/accrual-free core: re-age an
/// interest-recalculation loan through the real engine and rebuild the installment list from the re-aged model.
///
/// 1. `precalculateReAgeEqualAmortizationAmount` (ProgressiveEMICalculator 2070) =
///    `getOutstandingAmountsTillDate(PAYABLE ? txnDate : maturityDate)` → the transaction's principal/interest
///    components; a zero total reverses the transaction (src 3825-3827).
/// 2. the engine's `reAgeEqualAmortization` (already ported as
///    [`fc_loan_emi::recalc::Model::re_age_equal_amortization`]; the fee/penalty split arguments are zero here).
/// 3. the installment rebuild (src 3853-3913): drop the normal rows the re-age replaces (due on/after the
///    transaction) and additional rows absorbed inside the new maturity; re-anchor surviving additional rows at the
///    new maturity; for every model period either update the matching past row (charge := paid, so the row closes at
///    what was actually collected) or create a re-aged row carrying the engine's due principal / interest (the
///    early-repayment holder also carries its paid amounts); drop past rows no period claims; then
///    `reprocessInstallments` (src 3520) — sort by (from, due) and stitch each row's from-date to its predecessor's
///    due date.
#[must_use]
pub fn handle_re_age_progressive(
    model: &mut Model,
    installments: &mut Vec<AdvancedInstallment>,
    txn_date: Date,
    re_age_start_date: Date,
    number_of_installments: usize,
    frequency_months: i64,
    full_interest: bool,
) -> ReAgeOutcome {
    // 1. precalculate the re-age amounts (the transaction components).
    let target = if full_interest {
        model.periods.last().map_or(txn_date, |p| p.due)
    } else {
        txn_date
    };
    let (principal, interest) = model.get_outstanding_amounts_till_date(target);
    let reversed = principal.plus(&interest).is_zero();

    // 2. re-age the engine model.
    model.re_age_equal_amortization(
        txn_date,
        re_age_start_date,
        number_of_installments,
        frequency_months,
        full_interest,
    );
    let new_maturity = model.periods.last().map(|p| p.due);

    // 3a. drop replaced normal rows / absorbed additional rows; re-anchor surviving additional rows.
    installments.retain(|i| {
        let replaced_normal = !i.is_down_payment && !i.is_additional && i.due_date() >= txn_date;
        let absorbed_additional =
            i.is_additional && new_maturity.is_some_and(|m| i.due_date() <= m);
        !(replaced_normal || absorbed_additional)
    });
    if let Some(m) = new_maturity {
        for inst in installments.iter_mut().filter(|i| i.is_additional) {
            inst.set_from_date(m);
        }
    }

    // 3b. walk the re-aged model: update matched past rows, create re-aged rows for the rest.
    let mut matched = vec![false; installments.len()];
    let mut created: Vec<AdvancedInstallment> = Vec::new();
    for idx in 0..model.periods.len() {
        let (from, due) = (model.periods[idx].from, model.periods[idx].due);
        if due < txn_date {
            if let Some(pos) = installments
                .iter()
                .position(|i| i.due_date() == due && i.from_date() == from)
            {
                let completed = installments[pos].installment().principal_completed();
                let paid_interest = installments[pos].installment().interest_paid();
                installments[pos]
                    .installment_mut()
                    .update_principal(&completed);
                installments[pos]
                    .installment_mut()
                    .update_interest_charged(&paid_interest);
                installments[pos].set_credited_principal(&model.period_credited_principal(idx));
                matched[pos] = true;
            }
        } else {
            let mut row = AdvancedInstallment::new(
                Installment::new(
                    model.scale,
                    model.mode,
                    due,
                    model.period_due_principal(idx).amount().clone(),
                    model.period_due_interest(idx).amount().clone(),
                    BigDecimal::from(0),
                    BigDecimal::from(0),
                ),
                false,
            )
            .with_from_date(from);
            row.is_re_aged = true;
            if model.periods[idx].re_aged_early_repayment_holder {
                row.installment_mut()
                    .set_principal_completed(&model.periods[idx].paid_principal);
                row.installment_mut()
                    .set_interest_paid(&model.periods[idx].paid_interest);
            }
            row.set_credited_principal(&model.period_credited_principal(idx));
            created.push(row);
        }
    }

    // 3c. drop past normal rows no model period claimed, append the created rows, and reprocess.
    let mut keep = matched.into_iter();
    installments.retain(|i| {
        let m = keep.next().unwrap_or(true);
        i.is_additional || i.is_down_payment || m
    });
    installments.append(&mut created);
    reprocess_installments(installments);

    ReAgeOutcome {
        principal,
        interest,
        reversed,
    }
}

/// `reprocessInstallments` (src 3520): order by (from, due) and stitch each non-down-payment row's from-date to the
/// previous row's due date when discontinuous. (Installment numbers are positional here.)
fn reprocess_installments(installments: &mut [AdvancedInstallment]) {
    installments.sort_by_key(|i| (i.from_date(), i.due_date()));
    let mut previous_due: Option<Date> = None;
    for inst in installments.iter_mut() {
        if let Some(prev) = previous_due {
            if inst.from_date() != prev && !inst.is_down_payment {
                inst.set_from_date(prev);
            }
        }
        previous_due = Some(inst.due_date());
    }
}

/// The production [`EmiCalculator`] backed by the `fc-loan-emi` engine. Construct it from a fully-built
/// [`fc_loan_emi::recalc::Model`] (via that crate's constructors) and hand `&mut` it to the Advanced processor.
#[derive(Clone, Debug)]
pub struct ProgressiveEmiCalculator {
    model: Model,
}

impl ProgressiveEmiCalculator {
    /// Wrap a fully-constructed progressive interest schedule model.
    #[must_use]
    pub fn new(model: Model) -> Self {
        Self { model }
    }

    /// Borrow the underlying engine model (e.g. to read the resulting schedule after allocation).
    #[must_use]
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Consume the adapter, returning the underlying engine model.
    #[must_use]
    pub fn into_model(self) -> Model {
        self.model
    }
}

impl EmiCalculator for ProgressiveEmiCalculator {
    fn add_disbursement(&mut self, _model: &ScheduleModel, date: Date, amount: &Money) {
        self.model.add_disbursement(date, amount);
    }

    fn pay_principal(
        &mut self,
        _model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    ) {
        self.model
            .pay_principal(period_from, period_due, pay_date, amount);
    }

    fn pay_interest(
        &mut self,
        _model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    ) {
        self.model
            .pay_interest(period_from, period_due, pay_date, amount);
    }
}
