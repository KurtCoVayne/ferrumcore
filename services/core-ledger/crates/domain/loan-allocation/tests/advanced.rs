//! Conformance: replay the scenarios from the progressive processor's VERIFIED unit test
//! `AdvancedPaymentScheduleTransactionProcessorTest` (all 13 of its `@Test`s — golden baseline re-verified by running
//! the original on JDK 21) and assert the EXACT components / credited amounts / calculator interactions.
//!
//! Covered: the 3 `chargePayment*` tests; the 3 chargeback-credit tests (incl. the additional-installment one);
//! `calculateChargebackAllocationMap`; the 4 `findOriginalTransaction` tests; `testDisbursementAfterMaturityDate
//! WithEMICalculator`; and `testProcessLatestTransaction_PassesThroughHandlingPaymentAllocationForInterestBearing
//! ProgressiveLoan`. See `src/advanced.rs` for the (engine) scope notes.
//!
//! Currency USD/2, HALF_EVEN, matching the original.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::Installment;
use fc_loan_allocation::advanced::{
    AdvancedInstallment, AdvancedTransactionType, AllocationType, ChangedTransactionDetail,
    DueType, EmiCalculator, FutureInstallmentAllocationRule, LoanTxnView, OriginalAllocation,
    PaymentAllocation, RelationType, ScheduleModel, ScheduleProcessingType, TransactionRoute,
    TxnChange, TxnRelation, calculate_chargeback_allocation_map,
    find_chargeback_original_transaction, handle_charge_off, handle_charge_payment,
    handle_disbursement_with_emi, handle_payment_allocation_interest_bearing,
    handle_re_amortization, handle_refund, handle_write_off, process_credit_transaction,
    process_periods_horizontally, process_periods_vertically, process_repayment, route_transaction,
};
use fc_loan_allocation::charges::LoanChargeView;
use fc_money::Money;
use time::{Date, Month};

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn money(v: &str) -> Money {
    Money::of(
        SCALE,
        MODE,
        &BigDecimal::parse_bytes(v.as_bytes(), 10).unwrap(),
    )
}

fn date(y: i32, m: Month, d: u8) -> Date {
    Date::from_calendar_date(y, m, d).unwrap()
}

/// An installment carrying `fee` (and `penalty`) charged, all other components zero, due `due`.
fn charge_installment(due: Date, fee: &str, penalty: &str) -> AdvancedInstallment {
    let inst = Installment::new(
        SCALE,
        MODE,
        due,
        BigDecimal::from(0),
        BigDecimal::from(0),
        BigDecimal::parse_bytes(fee.as_bytes(), 10).unwrap(),
        BigDecimal::parse_bytes(penalty.as_bytes(), 10).unwrap(),
    );
    AdvancedInstallment::new(inst, false)
}

// ---------------------------------------------------------------------------------------------------------------------
// Charge payment
// ---------------------------------------------------------------------------------------------------------------------

/// `chargePaymentTransactionTestWithExactAmount`: charge 100 fully outstanding, txn 100 → fee portion 100,
/// updateComponents(0,0,100,0), unprocessed 0, fee outstanding 0.
#[test]
fn charge_payment_exact_amount() {
    let due = date(2023, Month::February, 1);
    let mut inst = charge_installment(due, "100", "0");
    let charge_outstanding = money("100");

    let r = handle_charge_payment(&mut inst, &money("100"), &charge_outstanding, false);

    assert_eq!(
        r.fee.to_plain_string(),
        money("100").to_plain_string(),
        "fee portion"
    );
    assert_eq!(
        r.penalty.to_plain_string(),
        money("0").to_plain_string(),
        "penalty portion"
    );
    assert_eq!(
        r.unprocessed.to_plain_string(),
        money("0").to_plain_string(),
        "unprocessed"
    );
    assert_eq!(
        inst.installment_mut()
            .fee_charges_outstanding()
            .to_plain_string(),
        money("0").to_plain_string(),
        "fee outstanding"
    );
}

/// `chargePaymentTransactionTestWithLessTransactionAmount`: charge 100 outstanding, txn 20 → fee portion 20,
/// updateComponents(0,0,20,0), unprocessed 0, fee outstanding 80.
#[test]
fn charge_payment_less_amount() {
    let due = date(2023, Month::February, 1);
    let mut inst = charge_installment(due, "100", "0");
    let charge_outstanding = money("100");

    let r = handle_charge_payment(&mut inst, &money("20"), &charge_outstanding, false);

    assert_eq!(
        r.fee.to_plain_string(),
        money("20").to_plain_string(),
        "fee portion"
    );
    assert_eq!(
        r.penalty.to_plain_string(),
        money("0").to_plain_string(),
        "penalty portion"
    );
    assert_eq!(
        r.unprocessed.to_plain_string(),
        money("0").to_plain_string(),
        "unprocessed"
    );
    assert_eq!(
        inst.installment_mut()
            .fee_charges_outstanding()
            .to_plain_string(),
        money("80").to_plain_string(),
        "fee outstanding"
    );
}

/// `chargePaymentTransactionTestWithMoreTransactionAmount` (charge-handling portion only): charge 100 outstanding,
/// txn 120 → amountToBePaid capped at 100, fee portion 100, updateComponents(0,0,100,0), unprocessed 20, fee
/// outstanding 0. The principal allocation of the 20 remainder runs in the out-of-scope `processTransaction` driver.
#[test]
fn charge_payment_more_amount_charge_portion() {
    let due = date(2023, Month::February, 1);
    let mut inst = charge_installment(due, "100", "0");
    let charge_outstanding = money("100");

    let r = handle_charge_payment(&mut inst, &money("120"), &charge_outstanding, false);

    assert_eq!(
        r.fee.to_plain_string(),
        money("100").to_plain_string(),
        "fee portion"
    );
    assert_eq!(
        r.penalty.to_plain_string(),
        money("0").to_plain_string(),
        "penalty portion"
    );
    assert_eq!(
        r.unprocessed.to_plain_string(),
        money("20").to_plain_string(),
        "unprocessed"
    );
    assert_eq!(
        inst.installment_mut()
            .fee_charges_outstanding()
            .to_plain_string(),
        money("0").to_plain_string(),
        "fee outstanding"
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Chargeback credit (credited-principal model)
// ---------------------------------------------------------------------------------------------------------------------

/// The original repayment's portions for the chargeback tests: `createRepayment(.., 10, 0, 20, 5)` →
/// principal 10, interest 0, fee 20, penalty 5.
fn original_10_0_20_5() -> OriginalAllocation {
    OriginalAllocation {
        principal: money("10"),
        interest: money("0"),
        fee: money("20"),
        penalty: money("5"),
    }
}

/// `testProcessCreditTransactionWithAllocationRulePrincipalPenaltyFeeInterest`: chargeback 25 over
/// [PRINCIPAL, PENALTY, FEE, INTEREST] with original (10,0,20,5) → components (P10, I0, F10, Pen5); the single due
/// installment is credited principal 10 (addToCreditedPrincipal + addToPrincipal 10), fee 10, penalty 5.
#[test]
fn credit_chargeback_principal_penalty_fee_interest() {
    let txn_date = date(2023, Month::January, 1);
    let original = original_10_0_20_5();
    let types = [
        AllocationType::Principal,
        AllocationType::Penalty,
        AllocationType::Fee,
        AllocationType::Interest,
    ];

    // The map-only golden first (mirrors the `calculateChargebackAllocationMap` unit assertions).
    let map = calculate_chargeback_allocation_map(&original, &money("25"), &types);
    assert_eq!(
        map.principal.to_plain_string(),
        money("10").to_plain_string(),
        "map principal"
    );
    assert_eq!(
        map.penalty.to_plain_string(),
        money("5").to_plain_string(),
        "map penalty"
    );
    assert_eq!(
        map.fee.to_plain_string(),
        money("10").to_plain_string(),
        "map fee"
    );
    assert_eq!(
        map.interest.to_plain_string(),
        money("0").to_plain_string(),
        "map interest"
    );

    let mut installments = [charge_installment(date(2023, Month::January, 31), "0", "0")];
    let r =
        process_credit_transaction(&original, &types, txn_date, &money("25"), &mut installments);

    // transaction components
    assert_eq!(
        r.components.principal.to_plain_string(),
        money("10").to_plain_string(),
        "txn principal"
    );
    assert_eq!(
        r.components.interest.to_plain_string(),
        money("0").to_plain_string(),
        "txn interest"
    );
    assert_eq!(
        r.components.fee.to_plain_string(),
        money("10").to_plain_string(),
        "txn fee"
    );
    assert_eq!(
        r.components.penalty.to_plain_string(),
        money("5").to_plain_string(),
        "txn penalty"
    );

    // installment credited amounts
    assert_eq!(
        r.credited_installment,
        Some(0),
        "credited installment index"
    );
    let inst = &installments[0];
    assert_eq!(
        inst.credited_principal().to_plain_string(),
        money("10").to_plain_string(),
        "credited principal"
    );
    assert_eq!(
        inst.added_principal().to_plain_string(),
        money("10").to_plain_string(),
        "addToPrincipal"
    );
    assert_eq!(
        inst.credited_fee().to_plain_string(),
        money("10").to_plain_string(),
        "credited fee"
    );
    assert_eq!(
        inst.credited_penalty().to_plain_string(),
        money("5").to_plain_string(),
        "credited penalty"
    );
    assert_eq!(
        inst.credited_interest().to_plain_string(),
        money("0").to_plain_string(),
        "credited interest"
    );
}

/// `testProcessCreditTransactionWithAllocationRulePenaltyFeePrincipalInterest`: chargeback 25 over
/// [PENALTY, FEE, PRINCIPAL, INTEREST] with original (10,0,20,5) → components (P0, I0, F20, Pen5); the due
/// installment is credited fee 20 and penalty 5 (no principal/interest credit).
#[test]
fn credit_chargeback_penalty_fee_principal_interest() {
    let txn_date = date(2023, Month::January, 1);
    let original = original_10_0_20_5();
    let types = [
        AllocationType::Penalty,
        AllocationType::Fee,
        AllocationType::Principal,
        AllocationType::Interest,
    ];

    let mut installments = [charge_installment(date(2023, Month::January, 31), "0", "0")];
    let r =
        process_credit_transaction(&original, &types, txn_date, &money("25"), &mut installments);

    assert_eq!(
        r.components.principal.to_plain_string(),
        money("0").to_plain_string(),
        "txn principal"
    );
    assert_eq!(
        r.components.interest.to_plain_string(),
        money("0").to_plain_string(),
        "txn interest"
    );
    assert_eq!(
        r.components.fee.to_plain_string(),
        money("20").to_plain_string(),
        "txn fee"
    );
    assert_eq!(
        r.components.penalty.to_plain_string(),
        money("5").to_plain_string(),
        "txn penalty"
    );

    assert_eq!(
        r.credited_installment,
        Some(0),
        "credited installment index"
    );
    let inst = &installments[0];
    assert_eq!(
        inst.credited_fee().to_plain_string(),
        money("20").to_plain_string(),
        "credited fee"
    );
    assert_eq!(
        inst.credited_penalty().to_plain_string(),
        money("5").to_plain_string(),
        "credited penalty"
    );
    assert_eq!(
        inst.credited_principal().to_plain_string(),
        money("0").to_plain_string(),
        "credited principal"
    );
    assert_eq!(
        inst.added_principal().to_plain_string(),
        money("0").to_plain_string(),
        "addToPrincipal"
    );
}

/// `testProcessCreditTransactionWithAllocationRulePrincipalAndInterestWithAdditionalInstallment`: two installments,
/// the first regular (due 2022-12-20, on/before the txn date) and the second ADDITIONAL (due 2022-12-27). The
/// chargeback (25 over [PRINCIPAL, PENALTY, FEE, INTEREST], original 10/0/20/5) credits the *additional* installment —
/// the first is skipped (not additional and not due after the 2023-01-01 transaction date).
#[test]
fn credit_chargeback_to_additional_installment() {
    let txn_date = date(2023, Month::January, 1);
    let original = original_10_0_20_5();
    let types = [
        AllocationType::Principal,
        AllocationType::Penalty,
        AllocationType::Fee,
        AllocationType::Interest,
    ];

    let zero_inst = |due| {
        Installment::new(
            SCALE,
            MODE,
            due,
            BigDecimal::from(0),
            BigDecimal::from(0),
            BigDecimal::from(0),
            BigDecimal::from(0),
        )
    };
    let mut installments = [
        AdvancedInstallment::new(zero_inst(date(2022, Month::December, 20)), false),
        AdvancedInstallment::new(zero_inst(date(2022, Month::December, 27)), true),
    ];

    let r =
        process_credit_transaction(&original, &types, txn_date, &money("25"), &mut installments);

    assert_eq!(
        r.credited_installment,
        Some(1),
        "credited installment index"
    );
    assert_eq!(
        r.components.principal.to_plain_string(),
        money("10").to_plain_string(),
        "txn principal"
    );
    assert_eq!(
        r.components.fee.to_plain_string(),
        money("10").to_plain_string(),
        "txn fee"
    );
    assert_eq!(
        r.components.penalty.to_plain_string(),
        money("5").to_plain_string(),
        "txn penalty"
    );
    assert_eq!(
        r.components.interest.to_plain_string(),
        money("0").to_plain_string(),
        "txn interest"
    );

    let inst = &installments[1];
    assert_eq!(
        inst.credited_principal().to_plain_string(),
        money("10").to_plain_string(),
        "credited principal"
    );
    assert_eq!(
        inst.added_principal().to_plain_string(),
        money("10").to_plain_string(),
        "addToPrincipal"
    );
    assert_eq!(
        inst.credited_fee().to_plain_string(),
        money("10").to_plain_string(),
        "credited fee"
    );
    assert_eq!(
        inst.credited_penalty().to_plain_string(),
        money("5").to_plain_string(),
        "credited penalty"
    );
}

/// Extra golden replaying the standalone `calculateChargebackAllocationMap` unit assertions (src line 641): the pure
/// distribution math used by the chargeback path.
#[test]
fn chargeback_allocation_map_unit_cases() {
    let types_p_i_f_pen = [
        AllocationType::Principal,
        AllocationType::Interest,
        AllocationType::Fee,
        AllocationType::Penalty,
    ];
    let types_p_f_pen_i = [
        AllocationType::Principal,
        AllocationType::Fee,
        AllocationType::Penalty,
        AllocationType::Interest,
    ];

    // (50,100,200,12), distribute 50, [P,I,F,Pen] → (50,0,0,0)
    let o = OriginalAllocation {
        principal: money("50"),
        interest: money("100"),
        fee: money("200"),
        penalty: money("12"),
    };
    let r = calculate_chargeback_allocation_map(&o, &money("50"), &types_p_i_f_pen);
    assert_eq!(r.principal.to_plain_string(), money("50").to_plain_string());
    assert_eq!(r.interest.to_plain_string(), money("0").to_plain_string());
    assert_eq!(r.fee.to_plain_string(), money("0").to_plain_string());
    assert_eq!(r.penalty.to_plain_string(), money("0").to_plain_string());

    // (40,100,200,12), distribute 50, [P,I,F,Pen] → (40,10,0,0)
    let o = OriginalAllocation {
        principal: money("40"),
        interest: money("100"),
        fee: money("200"),
        penalty: money("12"),
    };
    let r = calculate_chargeback_allocation_map(&o, &money("50"), &types_p_i_f_pen);
    assert_eq!(r.principal.to_plain_string(), money("40").to_plain_string());
    assert_eq!(r.interest.to_plain_string(), money("10").to_plain_string());
    assert_eq!(r.fee.to_plain_string(), money("0").to_plain_string());
    assert_eq!(r.penalty.to_plain_string(), money("0").to_plain_string());

    // (40,100,200,12), distribute 50, [P,F,Pen,I] → (40,0,10,0)
    let o = OriginalAllocation {
        principal: money("40"),
        interest: money("100"),
        fee: money("200"),
        penalty: money("12"),
    };
    let r = calculate_chargeback_allocation_map(&o, &money("50"), &types_p_f_pen_i);
    assert_eq!(r.principal.to_plain_string(), money("40").to_plain_string());
    assert_eq!(r.fee.to_plain_string(), money("10").to_plain_string());
    assert_eq!(r.penalty.to_plain_string(), money("0").to_plain_string());
    assert_eq!(r.interest.to_plain_string(), money("0").to_plain_string());

    // (40,100,200,12), distribute 340, [P,F,Pen,I] → (40,88,200,12)  [fee=200, pen=12, then I gets 340-40-200-12=88]
    let o = OriginalAllocation {
        principal: money("40"),
        interest: money("100"),
        fee: money("200"),
        penalty: money("12"),
    };
    let r = calculate_chargeback_allocation_map(&o, &money("340"), &types_p_f_pen_i);
    assert_eq!(r.principal.to_plain_string(), money("40").to_plain_string());
    assert_eq!(r.fee.to_plain_string(), money("200").to_plain_string());
    assert_eq!(r.penalty.to_plain_string(), money("12").to_plain_string());
    assert_eq!(r.interest.to_plain_string(), money("88").to_plain_string());

    // (40,100,200,12), distribute 352, [P,F,Pen,I] → (40,100,200,12)  [all original portions fully taken]
    let o = OriginalAllocation {
        principal: money("40"),
        interest: money("100"),
        fee: money("200"),
        penalty: money("12"),
    };
    let r = calculate_chargeback_allocation_map(&o, &money("352"), &types_p_f_pen_i);
    assert_eq!(r.principal.to_plain_string(), money("40").to_plain_string());
    assert_eq!(r.fee.to_plain_string(), money("200").to_plain_string());
    assert_eq!(r.penalty.to_plain_string(), money("12").to_plain_string());
    assert_eq!(r.interest.to_plain_string(), money("100").to_plain_string());
}

// ---------------------------------------------------------------------------------------------------------------------
// findChargebackOriginalTransaction
// ---------------------------------------------------------------------------------------------------------------------

fn txn(key: u64, id: Option<i64>, relations: Vec<TxnRelation>) -> LoanTxnView {
    LoanTxnView { key, id, relations }
}
/// A CHARGEBACK relation pointing to the transaction with key `to_key` / id `to_id`.
fn chargeback_rel(to_id: Option<i64>, to_key: u64) -> TxnRelation {
    TxnRelation {
        relation_type: RelationType::Chargeback,
        to_id,
        to_key,
    }
}

/// `testFindOriginalTransactionShouldFindOriginalInLoansTransactionWhenIdProvided`: the chargeback has id 123; the
/// repayment in the loan whose CHARGEBACK relation points to it is the original.
#[test]
fn find_original_by_id_in_loan_transactions() {
    let chargeback = txn(1, Some(123), vec![]);
    let repayment1 = txn(2, None, vec![]);
    let repayment2 = txn(3, None, vec![chargeback_rel(Some(123), 1)]);
    let loan_txns = [chargeback.clone(), repayment1, repayment2];

    let found = find_chargeback_original_transaction(&chargeback, &loan_txns, None);
    assert_eq!(found, Ok(3));
}

/// `testFindOriginalTransactionThrowsRuntimeExceptionWhenIdProvidedAndRelationsAreMissing`: no relation points to the
/// chargeback → the "must have an original transaction" error.
#[test]
fn find_original_missing_relations_errors() {
    let chargeback = txn(1, Some(123), vec![]);
    let repayment1 = txn(2, None, vec![]);
    let repayment2 = txn(3, None, vec![]);
    let loan_txns = [chargeback.clone(), repayment1, repayment2];

    let found = find_chargeback_original_transaction(&chargeback, &loan_txns, None);
    assert!(found.is_err());
    assert_eq!(
        found.unwrap_err().to_string(),
        "Chargeback transaction must have an original transaction"
    );
}

/// `testFindOriginalTransactionShouldFindOriginalInLoansTransactionFromTransactionCtxWhenIdIsNotProvided`: a replayed
/// chargeback has no id; its original id (123) is recovered from the change set, then the repayment whose relation
/// targets id 123 is found in the loan transactions.
#[test]
fn find_original_replayed_id_recovered_from_ctx() {
    let chargeback_replayed = txn(10, None, vec![]);
    let repayment1 = txn(2, None, vec![]);
    // repayment2 relates to the original chargeback (id 123, key 99).
    let repayment2 = txn(3, None, vec![chargeback_rel(Some(123), 99)]);
    let loan_txns = [repayment1, repayment2];

    let changed = ChangedTransactionDetail {
        changes: vec![TxnChange {
            old_id: Some(123),
            new: chargeback_replayed.clone(),
        }],
    };

    let found =
        find_chargeback_original_transaction(&chargeback_replayed, &loan_txns, Some(&changed));
    assert_eq!(found, Ok(3));
}

// ---------------------------------------------------------------------------------------------------------------------
// Disbursement after maturity (EMI calculator seam)
// ---------------------------------------------------------------------------------------------------------------------

/// A recording fake [`EmiCalculator`] that captures the interactions the original unit tests verify via Mockito.
#[derive(Default)]
struct RecordingCalc {
    disbursements: Vec<(Date, String)>,
    principal_pays: Vec<(Date, Date, Date, String)>,
}
impl EmiCalculator for RecordingCalc {
    fn add_disbursement(&mut self, _model: &ScheduleModel, date: Date, amount: &Money) {
        self.disbursements.push((date, amount.to_plain_string()));
    }
    fn pay_principal(
        &mut self,
        _model: &ScheduleModel,
        period_from: Date,
        period_due: Date,
        pay_date: Date,
        amount: &Money,
    ) {
        self.principal_pays
            .push((period_from, period_due, pay_date, amount.to_plain_string()));
    }
    fn pay_interest(
        &mut self,
        _model: &ScheduleModel,
        _period_from: Date,
        _period_due: Date,
        _pay_date: Date,
        _amount: &Money,
    ) {
    }
}

/// `testDisbursementAfterMaturityDateWithEMICalculator`: a 500 disbursement on 2023-07-15, after the last scheduled
/// installment's due date (2023-06-01). The EMI calculator receives `addDisbursement(model, 07-15, 500)`, and because
/// no non-additional installment is still due, an N+1 additional installment is created carrying principal 500 due
/// 07-15.
#[test]
fn disbursement_after_maturity_creates_additional_installment() {
    let mk = |due, principal: &str| {
        AdvancedInstallment::new(
            Installment::new(
                SCALE,
                MODE,
                due,
                BigDecimal::parse_bytes(principal.as_bytes(), 10).unwrap(),
                BigDecimal::from(10),
                BigDecimal::from(0),
                BigDecimal::from(0),
            ),
            false,
        )
    };
    let mut installments = vec![
        mk(date(2023, Month::February, 1), "200"),
        mk(date(2023, Month::March, 1), "200"),
        mk(date(2023, Month::June, 1), "600"),
    ];

    let model = ScheduleModel {
        maturity_date: date(2023, Month::December, 31),
    };
    let mut calc = RecordingCalc::default();
    let txn_date = date(2023, Month::July, 15);

    let r = handle_disbursement_with_emi(
        &mut calc,
        &model,
        &mut installments,
        SCALE,
        MODE,
        txn_date,
        &money("500"),
    );

    // EMI calculator received the disbursement.
    assert_eq!(calc.disbursements.len(), 1, "addDisbursement called once");
    assert_eq!(calc.disbursements[0].0, txn_date, "disbursement date");
    assert_eq!(
        calc.disbursements[0].1,
        money("500").to_plain_string(),
        "disbursement amount"
    );

    // An additional N+1 installment was created.
    let created = r
        .created_additional
        .expect("additional installment created");
    assert!(created.is_additional, "isAdditional");
    assert_eq!(
        created.due_date(),
        txn_date,
        "due date == disbursement date"
    );
    assert_eq!(
        created
            .installment()
            .principal_outstanding()
            .to_plain_string(),
        money("500").to_plain_string(),
        "principal == disbursement amount"
    );
    assert_eq!(installments.len(), 4, "installment appended to schedule");
}

/// `testProcessLatestTransaction_PassesThroughHandlingPaymentAllocationForInterestBearingProgressiveLoan`: for an
/// interest-bearing progressive loan, a DUE_PRINCIPAL allocation against the due installment pays the installment's
/// principal and then *passes through* to `emiCalculator.payPrincipal(model, fromDate, dueDate, payDate, paidPortion)`
/// with the full transaction amount. (Tested at the method the original test is named for; the surrounding
/// `processPeriodsHorizontally` installment-selection walk is the broader engine, out of scope.)
#[test]
fn payment_allocation_passthrough_pays_principal_via_emi_calculator() {
    let from = date(2023, Month::January, 1);
    let due = date(2023, Month::February, 1);
    let mut inst = AdvancedInstallment::new(
        Installment::new(
            SCALE,
            MODE,
            due,
            BigDecimal::from(100),
            BigDecimal::from(0),
            BigDecimal::from(100),
            BigDecimal::from(0),
        ),
        false,
    )
    .with_from_date(from);

    let model = ScheduleModel {
        maturity_date: date(2023, Month::December, 31),
    };
    let mut calc = RecordingCalc::default();
    let alloc = PaymentAllocation {
        allocation_type: AllocationType::Principal,
        due_type: DueType::Due,
    };

    // transaction date == due date (an on-time payment); the EMI period-from is the due installment's from date.
    let paid = handle_payment_allocation_interest_bearing(
        &mut calc,
        &model,
        &mut inst,
        alloc,
        due,
        &money("100"),
    );

    assert_eq!(
        paid.to_plain_string(),
        money("100").to_plain_string(),
        "paid portion"
    );
    assert_eq!(calc.principal_pays.len(), 1, "payPrincipal called once");
    let (period_from, period_due, pay_date, amount) = &calc.principal_pays[0];
    assert_eq!(*period_from, from, "period from date");
    assert_eq!(*period_due, due, "period due date");
    assert_eq!(*pay_date, due, "pay date == transaction date");
    assert_eq!(amount, &money("100").to_plain_string(), "paid amount");
    assert_eq!(
        inst.installment().principal_outstanding().to_plain_string(),
        money("0").to_plain_string(),
        "installment principal consumed"
    );
}

/// `...FallbackToPersistedTransactions`: same recovery, but the matching repayment is only in the persisted loan
/// transactions (not in the change set), so the lookup falls back to them.
#[test]
fn find_original_replayed_fallback_to_persisted() {
    let chargeback_replayed = txn(10, None, vec![]);
    let repayment1 = txn(2, None, vec![]);
    let repayment2 = txn(3, None, vec![chargeback_rel(Some(123), 99)]);
    let loan_txns = [repayment1, repayment2];

    let changed = ChangedTransactionDetail {
        changes: vec![TxnChange {
            old_id: Some(123),
            new: chargeback_replayed.clone(),
        }],
    };

    let found =
        find_chargeback_original_transaction(&chargeback_replayed, &loan_txns, Some(&changed));
    assert_eq!(found, Ok(3));
}

/// `processPeriodsHorizontally` non-interest-bearing DUE path — verified against the Java-app capture
/// (`fcCaptureNonInterestBearingHorizontalWalk`): one due installment (principal 100 + interest 10), a 150 payment
/// allocated `[DUE_PRINCIPAL, DUE_INTEREST]` → DUE_PRINCIPAL pays 100, DUE_INTEREST pays 10, the remaining 40 is the
/// overpayment (no future installment / no IN_ADVANCE type).
#[test]
fn horizontal_walk_due_then_overpayment() {
    let from = date(2023, Month::January, 1);
    let due = date(2023, Month::February, 1);
    let inst = Installment::new(
        SCALE,
        MODE,
        due,
        BigDecimal::from(100),
        BigDecimal::from(10),
        BigDecimal::from(0),
        BigDecimal::from(0),
    );
    let mut installments = [AdvancedInstallment::new(inst, false).with_from_date(from)];
    let allocations = [
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::Due,
        },
        PaymentAllocation {
            allocation_type: AllocationType::Interest,
            due_type: DueType::Due,
        },
    ];

    let overpayment = process_periods_horizontally(
        &mut installments,
        &allocations,
        FutureInstallmentAllocationRule::NextInstallment,
        due, // transaction on the due date
        &money("150"),
    );

    let paid = installments[0].installment();
    assert_eq!(paid.principal_outstanding().to_plain_string(), "0.00");
    assert_eq!(paid.interest_outstanding().to_plain_string(), "0.00");
    assert_eq!(paid.principal_completed().to_plain_string(), "100.00");
    assert_eq!(overpayment.to_plain_string(), "40.00");
}

/// IN_ADVANCE even-split — verified against the Java-app capture (`fcCaptureInAdvanceEvenSplit`, non-interest-bearing
/// `processAllocationsHorizontally` path, src 2826-2877). Three future installments (from/due one month apart), a
/// transaction on 2023-01-15 that precedes every due date, allocation `[IN_ADVANCE_PRINCIPAL]`. Covers the even split,
/// HALF_EVEN round-to-even (100.01/2 = 50.005 → 50.00) with the last installment absorbing the +0.01 residual, the
/// three-way remainder (33.33/33.33/33.34), and the NEXT/LAST/NEXT_LAST future-installment selection rules.
#[test]
fn horizontal_walk_in_advance_even_split() {
    let froms = [
        date(2023, Month::January, 1),
        date(2023, Month::February, 1),
        date(2023, Month::March, 1),
    ];
    let dues = [
        date(2023, Month::February, 1),
        date(2023, Month::March, 1),
        date(2023, Month::April, 1),
    ];
    let txn_date = date(2023, Month::January, 15);
    let allocations = [PaymentAllocation {
        allocation_type: AllocationType::Principal,
        due_type: DueType::InAdvance,
    }];

    let build = |count: usize| -> Vec<AdvancedInstallment> {
        (0..count)
            .map(|i| {
                let inst = Installment::new(
                    SCALE,
                    MODE,
                    dues[i],
                    BigDecimal::from(100),
                    BigDecimal::from(0),
                    BigDecimal::from(0),
                    BigDecimal::from(0),
                );
                AdvancedInstallment::new(inst, false).with_from_date(froms[i])
            })
            .collect()
    };

    let run = |rule, amount: &str, count: usize| -> (Vec<String>, String) {
        let mut installments = build(count);
        let overpayment = process_periods_horizontally(
            &mut installments,
            &allocations,
            rule,
            txn_date,
            &money(amount),
        );
        let principals = installments
            .iter()
            .map(|i| i.installment().principal_completed().to_plain_string())
            .collect();
        (principals, overpayment.to_plain_string())
    };

    // 1. REAMORTIZATION, 100.00 over 2 → 50.00 / 50.00.
    let (p, over) = run(FutureInstallmentAllocationRule::Reamortization, "100.00", 2);
    assert_eq!(p, ["50.00", "50.00"]);
    assert_eq!(over, "0.00");

    // 2. REAMORTIZATION, 100.01 over 2 → 50.00 / 50.01 (round-to-even + last absorbs +0.01).
    let (p, over) = run(FutureInstallmentAllocationRule::Reamortization, "100.01", 2);
    assert_eq!(p, ["50.00", "50.01"]);
    assert_eq!(over, "0.00");

    // 3. REAMORTIZATION, 100.00 over 3 → 33.33 / 33.33 / 33.34 (last absorbs the remainder).
    let (p, over) = run(FutureInstallmentAllocationRule::Reamortization, "100.00", 3);
    assert_eq!(p, ["33.33", "33.33", "33.34"]);
    assert_eq!(over, "0.00");

    // 4. NEXT_INSTALLMENT → earliest future installment takes it all.
    let (p, over) = run(FutureInstallmentAllocationRule::NextInstallment, "60.00", 2);
    assert_eq!(p, ["60.00", "0.00"]);
    assert_eq!(over, "0.00");

    // 5. LAST_INSTALLMENT → latest future installment takes it all.
    let (p, over) = run(FutureInstallmentAllocationRule::LastInstallment, "60.00", 2);
    assert_eq!(p, ["0.00", "60.00"]);
    assert_eq!(over, "0.00");

    // 6. NEXT_LAST_INSTALLMENT → the "current" installment (period contains the txn date) = the first.
    let (p, over) = run(
        FutureInstallmentAllocationRule::NextLastInstallment,
        "60.00",
        2,
    );
    assert_eq!(p, ["60.00", "0.00"]);
    assert_eq!(over, "0.00");
}

/// An installment with `principal`/`interest` charged (no fees), `from`/`due` a month apart.
fn pi_installment(from: Date, due: Date, principal: &str, interest: &str) -> AdvancedInstallment {
    let inst = Installment::new(
        SCALE,
        MODE,
        due,
        BigDecimal::parse_bytes(principal.as_bytes(), 10).unwrap(),
        BigDecimal::parse_bytes(interest.as_bytes(), 10).unwrap(),
        BigDecimal::from(0),
        BigDecimal::from(0),
    );
    AdvancedInstallment::new(inst, false).with_from_date(from)
}

/// The VERTICAL walk (`processPeriodsVertically`, src 3011) — verified against the Java-app capture
/// (`fcCaptureVerticalWalk`, non-interest-bearing path). Each allocation type is exhausted across the applicable
/// installments (its own do-while) before the next type. Covers PAST_DUE exhaust across two installments, the
/// IN_ADVANCE even-split (no DOWN guard), and a mixed PAST_DUE→DUE→IN_ADVANCE principal run.
#[test]
fn vertical_walk_matches_capture() {
    let froms = [
        date(2023, Month::January, 1),
        date(2023, Month::February, 1),
        date(2023, Month::March, 1),
    ];
    let dues = [
        date(2023, Month::February, 1),
        date(2023, Month::March, 1),
        date(2023, Month::April, 1),
    ];

    // V1: [PAST_DUE_PRINCIPAL, PAST_DUE_INTEREST] over two past-due installments (100/10 each), amount 250.
    // → both fully paid (P 100, I 10), 30 overpaid.
    let mut v1 = [
        pi_installment(froms[0], dues[0], "100", "10"),
        pi_installment(froms[1], dues[1], "100", "10"),
    ];
    let over = process_periods_vertically(
        &mut v1,
        &[
            PaymentAllocation {
                allocation_type: AllocationType::Principal,
                due_type: DueType::PastDue,
            },
            PaymentAllocation {
                allocation_type: AllocationType::Interest,
                due_type: DueType::PastDue,
            },
        ],
        FutureInstallmentAllocationRule::Reamortization,
        date(2023, Month::April, 1),
        &money("250"),
    );
    assert_eq!(
        v1[0].installment().principal_completed().to_plain_string(),
        "100.00"
    );
    assert_eq!(
        v1[0].installment().interest_outstanding().to_plain_string(),
        "0.00"
    );
    assert_eq!(
        v1[1].installment().principal_completed().to_plain_string(),
        "100.00"
    );
    assert_eq!(
        v1[1].installment().interest_outstanding().to_plain_string(),
        "0.00"
    );
    assert_eq!(over.to_plain_string(), "30.00");

    // V2: [IN_ADVANCE_PRINCIPAL] REAMORTIZATION, 100.01 over two future installments → 50.00 / 50.01.
    let mut v2 = [
        pi_installment(froms[0], dues[0], "100", "0"),
        pi_installment(froms[1], dues[1], "100", "0"),
    ];
    let over = process_periods_vertically(
        &mut v2,
        &[PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::InAdvance,
        }],
        FutureInstallmentAllocationRule::Reamortization,
        date(2023, Month::January, 15),
        &money("100.01"),
    );
    assert_eq!(
        v2[0].installment().principal_completed().to_plain_string(),
        "50.00"
    );
    assert_eq!(
        v2[1].installment().principal_completed().to_plain_string(),
        "50.01"
    );
    assert_eq!(over.to_plain_string(), "0.00");

    // V3: mixed [PAST_DUE, DUE, IN_ADVANCE] principal over three installments, txn on the 2nd's due date, amount 250
    // → 100 / 100 / 50.
    let mut v3 = [
        pi_installment(froms[0], dues[0], "100", "0"),
        pi_installment(froms[1], dues[1], "100", "0"),
        pi_installment(froms[2], dues[2], "100", "0"),
    ];
    let over = process_periods_vertically(
        &mut v3,
        &[
            PaymentAllocation {
                allocation_type: AllocationType::Principal,
                due_type: DueType::PastDue,
            },
            PaymentAllocation {
                allocation_type: AllocationType::Principal,
                due_type: DueType::Due,
            },
            PaymentAllocation {
                allocation_type: AllocationType::Principal,
                due_type: DueType::InAdvance,
            },
        ],
        FutureInstallmentAllocationRule::Reamortization,
        date(2023, Month::March, 1),
        &money("250"),
    );
    assert_eq!(
        v3[0].installment().principal_completed().to_plain_string(),
        "100.00"
    );
    assert_eq!(
        v3[1].installment().principal_completed().to_plain_string(),
        "100.00"
    );
    assert_eq!(
        v3[2].installment().principal_completed().to_plain_string(),
        "50.00"
    );
    assert_eq!(over.to_plain_string(), "0.00");
}

/// The top-level repayment entry (`processTransaction` → `processPeriods`, src 2648/2668) dispatches to the configured
/// walk orientation and returns the overpayment. Both arms run the IN_ADVANCE 100.01 split → 50.00 / 50.01.
#[test]
fn process_repayment_dispatches_both_orientations() {
    let froms = [
        date(2023, Month::January, 1),
        date(2023, Month::February, 1),
    ];
    let dues = [date(2023, Month::February, 1), date(2023, Month::March, 1)];
    let allocations = [PaymentAllocation {
        allocation_type: AllocationType::Principal,
        due_type: DueType::InAdvance,
    }];
    for pt in [
        ScheduleProcessingType::Horizontal,
        ScheduleProcessingType::Vertical,
    ] {
        let mut insts = [
            pi_installment(froms[0], dues[0], "100", "0"),
            pi_installment(froms[1], dues[1], "100", "0"),
        ];
        let over = process_repayment(
            &mut insts,
            &allocations,
            FutureInstallmentAllocationRule::Reamortization,
            pt,
            date(2023, Month::January, 15),
            &money("100.01"),
        );
        assert_eq!(
            insts[0]
                .installment()
                .principal_completed()
                .to_plain_string(),
            "50.00",
            "{pt:?} inst0"
        );
        assert_eq!(
            insts[1]
                .installment()
                .principal_completed()
                .to_plain_string(),
            "50.01",
            "{pt:?} inst1"
        );
        assert_eq!(over.to_plain_string(), "0.00", "{pt:?} overpayment");
    }
}

/// `route_transaction` faithfully mirrors the `processLatestTransaction` switch (src 564-582): the whole REPAYMENT
/// family folds onto `Repayment`, WAIVE_CHARGES is a logged no-op, every other type has its own handler.
#[test]
fn route_transaction_matches_java_switch() {
    use AdvancedTransactionType as T;
    let cases = [
        (T::Disbursement, TransactionRoute::Disbursement),
        (T::WriteOff, TransactionRoute::WriteOff),
        (T::RefundForActiveLoan, TransactionRoute::Refund),
        (T::Chargeback, TransactionRoute::Chargeback),
        (
            T::CreditBalanceRefund,
            TransactionRoute::CreditBalanceRefund,
        ),
        (T::Repayment, TransactionRoute::Repayment),
        (T::MerchantIssuedRefund, TransactionRoute::Repayment),
        (T::PayoutRefund, TransactionRoute::Repayment),
        (T::GoodwillCredit, TransactionRoute::Repayment),
        (T::ChargeRefund, TransactionRoute::Repayment),
        (T::ChargeAdjustment, TransactionRoute::Repayment),
        (T::DownPayment, TransactionRoute::Repayment),
        (T::WaiveInterest, TransactionRoute::Repayment),
        (T::RecoveryRepayment, TransactionRoute::Repayment),
        (T::InterestPaymentWaiver, TransactionRoute::Repayment),
        (T::CapitalizedIncomeAdjustment, TransactionRoute::Repayment),
        (T::InterestRefund, TransactionRoute::InterestRefund),
        (T::ChargeOff, TransactionRoute::ChargeOff),
        (T::ChargePayment, TransactionRoute::ChargePayment),
        (T::WaiveCharges, TransactionRoute::WaiveChargesNoop),
        (T::ReAmortize, TransactionRoute::ReAmortize),
        (T::ReAge, TransactionRoute::ReAge),
        (T::CapitalizedIncome, TransactionRoute::CapitalizedIncome),
        (
            T::ContractTermination,
            TransactionRoute::ContractTermination,
        ),
    ];
    for (t, expected) in cases {
        assert_eq!(route_transaction(t), expected, "route for {t:?}");
    }
}

/// `handleWriteOff` (src 1927 → super 764) — verified against the Java-app capture (`fcCaptureWriteOff`): two
/// not-fully-paid installments (100/10/5/3 and 50/4/0/0) written off → aggregate P 150, I 14, F 5, Pen 3, and every
/// installment's outstanding driven to zero.
#[test]
fn write_off_matches_capture() {
    let inst1 = AdvancedInstallment::new(
        Installment::new(
            SCALE,
            MODE,
            date(2023, Month::February, 1),
            BigDecimal::from(100),
            BigDecimal::from(10),
            BigDecimal::from(5),
            BigDecimal::from(3),
        ),
        false,
    )
    .with_from_date(date(2023, Month::January, 1));
    let inst2 = AdvancedInstallment::new(
        Installment::new(
            SCALE,
            MODE,
            date(2023, Month::March, 1),
            BigDecimal::from(50),
            BigDecimal::from(4),
            BigDecimal::from(0),
            BigDecimal::from(0),
        ),
        false,
    )
    .with_from_date(date(2023, Month::February, 1));
    let mut installments = [inst1, inst2];

    let wo = handle_write_off(&mut installments);

    assert_eq!(wo.principal.to_plain_string(), "150.00");
    assert_eq!(wo.interest.to_plain_string(), "14.00");
    assert_eq!(wo.fee.to_plain_string(), "5.00");
    assert_eq!(wo.penalty.to_plain_string(), "3.00");
    for inst in &installments {
        let i = inst.installment();
        assert_eq!(i.principal_outstanding().to_plain_string(), "0.00");
        assert_eq!(i.interest_outstanding().to_plain_string(), "0.00");
        assert_eq!(i.fee_charges_outstanding().to_plain_string(), "0.00");
        assert_eq!(i.penalty_charges_outstanding().to_plain_string(), "0.00");
    }
}

/// `handleReAmortization` → `reamortizationOnNonInterestBearingLoan` (src 699/805) — verified against the Java-app
/// capture (`fcCaptureReAmortization`): five 100-principal installments due Feb–Jun, transaction on Mar 1 (so #1 and
/// #2 are "previous" — due on or before). Covers the overdue sweep (previous principal set to completed), the even
/// redistribution with the latest-due row absorbing the remainder, the `installmentAmountInMultiplesOf` rounding, and
/// the zero-overdue reversal.
#[test]
fn re_amortization_matches_capture() {
    let txn_date = date(2023, Month::March, 1);
    let build = || -> Vec<AdvancedInstallment> {
        (0..5)
            .map(|i| {
                let months = [
                    (Month::January, Month::February),
                    (Month::February, Month::March),
                    (Month::March, Month::April),
                    (Month::April, Month::May),
                    (Month::May, Month::June),
                ];
                let (fm, dm) = months[i];
                pi_installment(date(2023, fm, 1), date(2023, dm, 1), "100", "0")
            })
            .collect()
    };

    // R1: 40 paid on #1, nothing on #2 → overdue 160 spread over #3/#4/#5 as 53.33/53.33/53.34.
    let mut r1 = build();
    let _ = r1[0].installment_mut().pay_principal(&money("40"));
    let out = handle_re_amortization(&mut r1, txn_date, None);
    assert!(!out.reversed);
    assert_eq!(out.principal.to_plain_string(), "160.00");
    let principals: Vec<String> = r1
        .iter()
        .map(|i| i.installment().principal().to_plain_string())
        .collect();
    assert_eq!(principals, ["40.00", "0.00", "153.33", "153.33", "153.34"]);

    // R2: everything previous fully paid → overdue 0 → the transaction reverses, schedule untouched.
    let mut r2 = build();
    let _ = r2[0].installment_mut().pay_principal(&money("100"));
    let _ = r2[1].installment_mut().pay_principal(&money("100"));
    let out = handle_re_amortization(&mut r2, txn_date, None);
    assert!(out.reversed);
    assert_eq!(out.principal.to_plain_string(), "0.00");
    let principals: Vec<String> = r2
        .iter()
        .map(|i| i.installment().principal().to_plain_string())
        .collect();
    assert_eq!(
        principals,
        ["100.00", "100.00", "100.00", "100.00", "100.00"]
    );

    // R3: as R1 with installmentAmountInMultiplesOf = 10 → per 53.33 rounds to 50, latest-due absorbs 60.
    let mut r3 = build();
    let _ = r3[0].installment_mut().pay_principal(&money("40"));
    let out = handle_re_amortization(&mut r3, txn_date, Some(10));
    assert!(!out.reversed);
    assert_eq!(out.principal.to_plain_string(), "160.00");
    let principals: Vec<String> = r3
        .iter()
        .map(|i| i.installment().principal().to_plain_string())
        .collect();
    assert_eq!(principals, ["40.00", "0.00", "150.00", "150.00", "160.00"]);
}

/// `handleRefund` (src 1374) — verified against the Java-app capture (`fcCaptureRefund`).
///
/// F1 (horizontal, DEFAULT rule): dues Feb/Mar/Apr paid 100/60/40, refund 130 on Mar 15. The default rule
/// `[PAST_DUE_P, DUE_P, IN_ADVANCE_P]` reverses to `[IN_ADVANCE_P, DUE_P, PAST_DUE_P]` with LAST_INSTALLMENT:
/// IN_ADVANCE unpays 40 off #3, DUE finds nothing, PAST_DUE unpays latest-first (60 off #2, then 30 off #1)
/// → 70 / 0 / 0.
///
/// F2 (vertical, custom rule + REAMORTIZATION): dues Feb/Apr/May paid 20/50/50, refund 120.01 on Mar 15. PAST_DUE
/// unpays 20 off #1; IN_ADVANCE splits 100.01 → 50.00 / 50.01, both capped at the 50 paid → 0 / 0 / 0 with 0.01
/// unallocatable.
#[test]
fn refund_matches_capture() {
    let txn_date = date(2023, Month::March, 15);

    // F1: horizontal, default rule.
    let mut f1 = [
        pi_installment(
            date(2023, Month::January, 1),
            date(2023, Month::February, 1),
            "100",
            "0",
        ),
        pi_installment(
            date(2023, Month::February, 1),
            date(2023, Month::March, 1),
            "100",
            "0",
        ),
        pi_installment(
            date(2023, Month::March, 1),
            date(2023, Month::April, 1),
            "100",
            "0",
        ),
    ];
    let _ = f1[0].installment_mut().pay_principal(&money("100"));
    let _ = f1[1].installment_mut().pay_principal(&money("60"));
    let _ = f1[2].installment_mut().pay_principal(&money("40"));
    let allocations = [
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::PastDue,
        },
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::Due,
        },
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::InAdvance,
        },
    ];
    let unprocessed = handle_refund(
        &mut f1,
        &allocations,
        FutureInstallmentAllocationRule::NextInstallment, // ignored: the default rule forces LAST_INSTALLMENT
        ScheduleProcessingType::Horizontal,
        txn_date,
        &money("130"),
        true,
    );
    let done: Vec<String> = f1
        .iter()
        .map(|i| i.installment().principal_completed().to_plain_string())
        .collect();
    assert_eq!(done, ["70.00", "0.00", "0.00"]);
    assert_eq!(unprocessed.to_plain_string(), "0.00");

    // F2: vertical, custom rule with REAMORTIZATION.
    let mut f2 = [
        pi_installment(
            date(2023, Month::January, 1),
            date(2023, Month::February, 1),
            "100",
            "0",
        ),
        pi_installment(
            date(2023, Month::March, 1),
            date(2023, Month::April, 1),
            "100",
            "0",
        ),
        pi_installment(
            date(2023, Month::April, 1),
            date(2023, Month::May, 1),
            "100",
            "0",
        ),
    ];
    let _ = f2[0].installment_mut().pay_principal(&money("20"));
    let _ = f2[1].installment_mut().pay_principal(&money("50"));
    let _ = f2[2].installment_mut().pay_principal(&money("50"));
    let allocations = [
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::PastDue,
        },
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::InAdvance,
        },
    ];
    let unprocessed = handle_refund(
        &mut f2,
        &allocations,
        FutureInstallmentAllocationRule::Reamortization,
        ScheduleProcessingType::Vertical,
        txn_date,
        &money("120.01"),
        false,
    );
    let done: Vec<String> = f2
        .iter()
        .map(|i| i.installment().principal_completed().to_plain_string())
        .collect();
    assert_eq!(done, ["0.00", "0.00", "0.00"]);
    assert_eq!(unprocessed.to_plain_string(), "0.01");
}

/// `handleChargeOff` (src 2082, REGULAR / non-interest-bearing) — verified against the Java-app capture
/// (`fcCaptureChargeOff`): 100/10/5/3 (30 principal paid) + 50/4/0/0 → components sweep the outstanding
/// (P 120, I 14, F 5, Pen 3) WITHOUT touching the installments; the fully-paid variant reverses and lifts.
#[test]
fn charge_off_matches_capture() {
    // C1: outstanding remains.
    let mut inst1 = AdvancedInstallment::new(
        Installment::new(
            SCALE,
            MODE,
            date(2023, Month::February, 1),
            BigDecimal::from(100),
            BigDecimal::from(10),
            BigDecimal::from(5),
            BigDecimal::from(3),
        ),
        false,
    )
    .with_from_date(date(2023, Month::January, 1));
    let _ = inst1.installment_mut().pay_principal(&money("30"));
    let inst2 = AdvancedInstallment::new(
        Installment::new(
            SCALE,
            MODE,
            date(2023, Month::March, 1),
            BigDecimal::from(50),
            BigDecimal::from(4),
            BigDecimal::from(0),
            BigDecimal::from(0),
        ),
        false,
    )
    .with_from_date(date(2023, Month::February, 1));
    let installments = [inst1, inst2];

    let out = handle_charge_off(&installments);
    assert_eq!(out.components.principal.to_plain_string(), "120.00");
    assert_eq!(out.components.interest.to_plain_string(), "14.00");
    assert_eq!(out.components.fee.to_plain_string(), "5.00");
    assert_eq!(out.components.penalty.to_plain_string(), "3.00");
    assert!(!out.reversed);
    assert!(out.charged_off);
    // Charge-off does NOT mutate the installments (unlike write-off).
    assert_eq!(
        installments[0]
            .installment()
            .principal_outstanding()
            .to_plain_string(),
        "70.00"
    );

    // C2: everything paid → all components zero → reverse + lift.
    let mut paid = installments;
    {
        let i = paid[0].installment_mut();
        let _ = i.pay_principal(&money("70"));
        let _ = i.pay_interest(&money("10"));
        let _ = i.pay_fee(&money("5"));
        let _ = i.pay_penalty(&money("3"));
    }
    {
        let i = paid[1].installment_mut();
        let _ = i.pay_principal(&money("50"));
        let _ = i.pay_interest(&money("4"));
    }
    let out = handle_charge_off(&paid);
    assert_eq!(out.components.principal.to_plain_string(), "0.00");
    assert!(out.reversed);
    assert!(!out.charged_off);
}

/// The FEE leaf's charge distribution wired through the horizontal walk — replays the captured Java golden
/// (`fcCaptureChargeDistribution`): one installment due on the txn date carrying fee 15 backed by three fee charges
/// A (due Feb 10, 4 with 1 pre-paid), B (due Jan 5, 5), C (due Feb 20, 7); a 12 payment allocated `[DUE_FEE]`
/// distributes earliest-due-first B 5 → A 3 → C 4, with matching `LoanChargePaidBy` records stamped with the
/// installment number.
#[test]
fn walk_fee_leaf_distributes_charges() {
    let txn_date = date(2023, Month::March, 1);
    let mut a = LoanChargeView::new(money("4"), Some(date(2023, Month::February, 10)), false);
    let _ = a.update_paid_amount_by(&money("1"));
    let b = LoanChargeView::new(money("5"), Some(date(2023, Month::January, 5)), false);
    let c = LoanChargeView::new(money("7"), Some(date(2023, Month::February, 20)), false);

    let inst = Installment::new(
        SCALE,
        MODE,
        txn_date,
        BigDecimal::from(100),
        BigDecimal::from(0),
        BigDecimal::from(15),
        BigDecimal::from(0),
    );
    let mut installments = [AdvancedInstallment::new(inst, false)
        .with_from_date(date(2023, Month::February, 1))
        .with_installment_number(1)
        .with_charges(vec![a, b, c])];

    let overpayment = process_periods_horizontally(
        &mut installments,
        &[PaymentAllocation {
            allocation_type: AllocationType::Fee,
            due_type: DueType::Due,
        }],
        FutureInstallmentAllocationRule::NextInstallment,
        txn_date,
        &money("12"),
    );

    assert_eq!(overpayment.to_plain_string(), "0.00");
    assert_eq!(
        installments[0]
            .installment()
            .fee_charges_outstanding()
            .to_plain_string(),
        "3.00"
    );
    let records: Vec<(usize, String, Option<u32>)> = installments[0]
        .charges_paid_by()
        .iter()
        .map(|r| (r.charge, r.amount.to_plain_string(), r.installment_number))
        .collect();
    assert_eq!(
        records,
        [
            (1, "5.00".to_string(), Some(1)),
            (0, "3.00".to_string(), Some(1)),
            (2, "4.00".to_string(), Some(1)),
        ]
    );
    // The charge entities' outstanding reflect the distribution (B/A fully paid, C 3 remaining).
    assert_eq!(
        installments[0].charges()[1]
            .amount_outstanding
            .to_plain_string(),
        "0.00"
    );
    assert_eq!(
        installments[0].charges()[2]
            .amount_outstanding
            .to_plain_string(),
        "3.00"
    );
}
