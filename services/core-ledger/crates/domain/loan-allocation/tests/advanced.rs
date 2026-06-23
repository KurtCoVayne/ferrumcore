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
    AdvancedInstallment, AllocationType, ChangedTransactionDetail, DueType, EmiCalculator,
    LoanTxnView, OriginalAllocation, PaymentAllocation, RelationType, ScheduleModel, TxnChange,
    TxnRelation, calculate_chargeback_allocation_map, find_chargeback_original_transaction,
    handle_charge_payment, handle_disbursement_with_emi,
    handle_payment_allocation_interest_bearing, process_credit_transaction,
};
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
