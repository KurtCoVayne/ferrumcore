//! Characterization of each legacy strategy's refund ordering (`handleRefundTransactionPaymentOfInstallment`).
//!
//! The original processors do NOT all share one refund order: five use principal → interest → fee → penalty, while
//! EarlyPayment and InterestPrincipalPenaltyFeesOrder use fee → penalty → principal → interest, HeavensFamily uses
//! penalty → fee → interest → principal, and PrincipalInterestPenaltyFeesOrder uses fee → penalty → interest →
//! principal. These per-strategy orders were extracted from the Java refund handlers; the expected tuples below are
//! hand-traced through the golden-verified `unpay_*` atoms.
//!
//! Method: fully pay an installment (P5 I4 F3 Pen2, total 14) so every paid bucket is populated, then refund and assert
//! which buckets are reversed first. `refund(1)` pins the first bucket; `refund(11)` pins the deeper ordering.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::creocore::Creocore;
use fc_loan_allocation::strategy::due_pen_int_pri_fee::DuePenIntPriFee;
use fc_loan_allocation::strategy::early_payment::EarlyPayment;
use fc_loan_allocation::strategy::heavens_family::HeavensFamily;
use fc_loan_allocation::strategy::interest_principal_penalty_fees_order::InterestPrincipalPenaltyFeesOrder;
use fc_loan_allocation::strategy::principal_interest_penalty_fees_order::PrincipalInterestPenaltyFeesOrder;
use fc_loan_allocation::strategy::{allocate_on_time, allocate_refund};
use fc_loan_allocation::{Allocation, Installment, Strategy, Txn};
use fc_money::Money;
use time::{Date, Month};

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn date() -> Date {
    Date::from_calendar_date(2023, Month::July, 31).unwrap()
}
fn money(v: i64) -> Money {
    Money::of(SCALE, MODE, &BigDecimal::from(v))
}

/// A P5 I4 F3 Pen2 installment with every component fully paid (via an on-time payment of the 14 total).
fn fully_paid(strategy: &dyn Strategy) -> Installment {
    let mut inst = Installment::new(
        SCALE,
        MODE,
        date(),
        BigDecimal::from(5),
        BigDecimal::from(4),
        BigDecimal::from(3),
        BigDecimal::from(2),
    );
    let paid = allocate_on_time(strategy, &mut inst, &Txn::repayment(date(), money(14)));
    assert_eq!(
        paid.unprocessed.to_plain_string(),
        money(0).to_plain_string()
    );
    inst
}

fn assert_alloc(label: &str, got: &Allocation, p: i64, i: i64, f: i64, pen: i64, unproc: i64) {
    let c = |n: &str, a: &Money, e: i64| {
        assert_eq!(
            a.to_plain_string(),
            money(e).to_plain_string(),
            "{label}: {n}"
        );
    };
    c("principal", &got.principal, p);
    c("interest", &got.interest, i);
    c("fee", &got.fee, f);
    c("penalty", &got.penalty, pen);
    c("unprocessed", &got.unprocessed, unproc);
}

/// refund(1) then refund(11) on a freshly fully-paid installment, asserting both tuples.
fn check(strategy: &dyn Strategy, label: &str, r1: [i64; 5], r11: [i64; 5]) {
    let mut inst = fully_paid(strategy);
    let a1 = allocate_refund(strategy, &mut inst, &money(1));
    assert_alloc(
        &format!("{label}/refund1"),
        &a1,
        r1[0],
        r1[1],
        r1[2],
        r1[3],
        r1[4],
    );

    let mut inst = fully_paid(strategy);
    let a11 = allocate_refund(strategy, &mut inst, &money(11));
    assert_alloc(
        &format!("{label}/refund11"),
        &a11,
        r11[0],
        r11[1],
        r11[2],
        r11[3],
        r11[4],
    );
}

#[test]
fn common_order_principal_interest_fee_penalty() {
    // principal → interest → fee → penalty. refund1 → principal 1; refund11 → principal 5, interest 4, fee 2.
    check(
        &DuePenIntPriFee,
        "due_pen_int_pri_fee",
        [1, 0, 0, 0, 0],
        [5, 4, 2, 0, 0],
    );
    check(&Creocore, "creocore", [1, 0, 0, 0, 0], [5, 4, 2, 0, 0]);
}

#[test]
fn early_payment_fee_penalty_principal_interest() {
    // fee → penalty → principal → interest. refund1 → fee 1; refund11 → fee 3, penalty 2, principal 5, interest 1.
    check(
        &EarlyPayment,
        "early_payment",
        [0, 0, 1, 0, 0],
        [5, 1, 3, 2, 0],
    );
    check(
        &InterestPrincipalPenaltyFeesOrder,
        "interest_principal_penalty_fees_order",
        [0, 0, 1, 0, 0],
        [5, 1, 3, 2, 0],
    );
}

#[test]
fn principal_interest_penalty_fees_order_fee_penalty_interest_principal() {
    // fee → penalty → interest → principal. refund1 → fee 1; refund11 → fee 3, penalty 2, interest 4, principal 2.
    check(
        &PrincipalInterestPenaltyFeesOrder,
        "principal_interest_penalty_fees_order",
        [0, 0, 1, 0, 0],
        [2, 4, 3, 2, 0],
    );
}

#[test]
fn heavens_family_penalty_fee_interest_principal() {
    // penalty → fee → interest → principal. refund1 → penalty 1; refund11 → penalty 2, fee 3, interest 4, principal 2.
    check(
        &HeavensFamily,
        "heavens_family",
        [0, 0, 0, 1, 0],
        [2, 4, 3, 2, 0],
    );
}
