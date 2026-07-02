//! Conformance: the charge-payment distribution against the captured Java golden (`fcCaptureChargeDistribution`,
//! JDK 21 — the walk's FEE leaf driving `updateChargesPaidAmountBy` over stateful charges) plus the
//! transcription-locked `LoanCharge` pay/undo arithmetic and the due-in-period filter.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::charges::{
    LoanChargeView, charges_due_in_period, undo_charges_paid_amount_by,
    update_charges_paid_amount_by,
};
use fc_money::Money;
use std::str::FromStr;
use time::{Date, Month};

fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &BigDecimal::from_str(s).unwrap())
}

fn d(y: i32, mo: Month, day: u8) -> Date {
    Date::from_calendar_date(y, mo, day).unwrap()
}

/// The FCCHG golden: charges A (due Feb 10, outstanding 3 with 1 already paid), B (due Jan 5, 5), C (due Feb 20, 7);
/// a 12 fee payment distributes earliest-due-first — B 5, then A 3, then C 4 — with matching positive
/// `LoanChargePaidBy` records; the refund reverses latest-paid-first (C, then A, then B) with negative records.
#[test]
fn distribution_matches_java_capture() {
    let feb10 = d(2023, Month::February, 10);
    let jan5 = d(2023, Month::January, 5);
    let feb20 = d(2023, Month::February, 20);

    let mut a = LoanChargeView::new(m("4"), Some(feb10), false);
    // A is partially paid already (1 of 4).
    let _ = a.update_paid_amount_by(&m("1"));
    let b = LoanChargeView::new(m("5"), Some(jan5), false);
    let c = LoanChargeView::new(m("7"), Some(feb20), false);
    let mut charges = [a, b, c];

    let records = update_charges_paid_amount_by(&mut charges, &m("12"), Some(1));

    // Golden: PAY B 5; PAY A 3; PAY C 4 — chargesPaidBy A=3.00, B=5.00, C=4.00.
    let seq: Vec<(usize, String)> = records
        .iter()
        .map(|r| (r.charge, r.amount.to_plain_string()))
        .collect();
    assert_eq!(
        seq,
        [
            (1, "5.00".to_string()),
            (0, "3.00".to_string()),
            (2, "4.00".to_string())
        ]
    );
    assert!(records.iter().all(|r| r.installment_number == Some(1)));
    assert!(charges[0].paid);
    assert!(charges[1].paid);
    assert!(!charges[2].paid);
    assert_eq!(charges[2].amount_outstanding.to_plain_string(), "3.00");
    assert_eq!(charges[2].amount_paid.to_plain_string(), "4.00");

    // Refund 6 reverses latest-paid-first: C (4), then A (2 of its 4 paid).
    let undo = undo_charges_paid_amount_by(&mut charges, &m("6"));
    let seq: Vec<(usize, String)> = undo
        .iter()
        .map(|r| (r.charge, r.amount.to_plain_string()))
        .collect();
    assert_eq!(seq, [(2, "-4.00".to_string()), (0, "-2.00".to_string())]);
    assert!(undo.iter().all(|r| r.installment_number.is_none()));
    assert!(!charges[2].paid);
    assert_eq!(charges[2].amount_outstanding.to_plain_string(), "7.00");
    assert_eq!(charges[0].amount_paid.to_plain_string(), "2.00");
    assert_eq!(charges[0].amount_outstanding.to_plain_string(), "2.00");

    // Refund beyond what was paid terminates (Java would spin — documented divergence) after draining everything.
    let drained = undo_charges_paid_amount_by(&mut charges, &m("100"));
    let total_undone = drained.iter().fold(m("0"), |acc, r| acc.plus(&r.amount));
    assert_eq!(
        total_undone.to_plain_string(),
        "-7.00",
        "2 (A) + 5 (B) remaining paid"
    );
}

/// `isDueInPeriod` (from-exclusive due-inclusive; first period inclusive-inclusive) over the charge list — the
/// regular-charge selection of `getLoanChargesOfInstallment`.
#[test]
fn due_in_period_filter() {
    let feb1 = d(2023, Month::February, 1);
    let mar1 = d(2023, Month::March, 1);
    let charges = [
        LoanChargeView::new(m("1"), Some(feb1), false), // on the from date
        LoanChargeView::new(m("1"), Some(d(2023, Month::February, 15)), true), // inside
        LoanChargeView::new(m("1"), Some(mar1), false), // on the due date
        LoanChargeView::new(m("1"), Some(d(2023, Month::March, 2)), false), // after
        LoanChargeView::new(m("1"), None, false),       // no due date
    ];
    assert_eq!(charges_due_in_period(&charges, feb1, mar1, false), [1, 2]);
    assert_eq!(charges_due_in_period(&charges, feb1, mar1, true), [0, 1, 2]);
}
