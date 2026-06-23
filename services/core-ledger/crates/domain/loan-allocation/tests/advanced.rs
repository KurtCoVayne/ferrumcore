//! Conformance: replay the in-scope scenarios from the progressive processor's VERIFIED unit test
//! `AdvancedPaymentScheduleTransactionProcessorTest` and assert the EXACT components/credited amounts.
//!
//! Covered (see `src/advanced.rs` for scope notes):
//!  - `chargePaymentTransactionTestWithExactAmount`     — a charge fully paid.
//!  - `chargePaymentTransactionTestWithLessTransactionAmount` — a charge partially paid.
//!  - `chargePaymentTransactionTestWithMoreTransactionAmount` — mixed charge + (unprocessed) principal: only the
//!    charge-handling portion is asserted here (the principal allocation of the 20 remainder runs in the
//!    out-of-scope `processTransaction` driver).
//!  - `testProcessCreditTransactionWithAllocationRulePrincipalPenaltyFeeInterest` and
//!    `testProcessCreditTransactionWithAllocationRulePenaltyFeePrincipalInterest` — chargeback crediting principal
//!    (the credited-principal model).
//!
//! Currency USD/2, HALF_EVEN, matching the original.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::Installment;
use fc_loan_allocation::advanced::{
    AdvancedInstallment, AllocationType, OriginalAllocation, calculate_chargeback_allocation_map,
    handle_charge_payment, process_credit_transaction,
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
