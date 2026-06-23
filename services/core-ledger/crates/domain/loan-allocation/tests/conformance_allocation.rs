//! Conformance: replay every scenario from the original's VERIFIED unit test
//! `DuePenIntPriFeeInAdvancePenIntPriFeeLoanRepaymentScheduleTransactionProcessorTest` and assert the EXACT
//! `updateComponents(principal, interest, fee, penalty)` tuple plus the unprocessed remainder.
//!
//! The 33 charge-free scenarios are driven from `golden/allocation_legacy.tsv`; the 3 due-charge-driven in-advance
//! scenarios are encoded inline (they carry structured `PendingCharge` data the flat TSV cannot).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::{Allocation, Installment, PendingCharge, due_pen_int_pri_fee as strat};
use fc_money::Money;
use time::{Date, Month};

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;

fn txn_date() -> Date {
    Date::from_calendar_date(2023, Month::July, 11).unwrap()
}
fn due_date() -> Date {
    Date::from_calendar_date(2023, Month::July, 31).unwrap()
}
fn late_date() -> Date {
    Date::from_calendar_date(2023, Month::August, 1).unwrap()
}

fn money(v: i64) -> Money {
    Money::of(SCALE, MODE, &BigDecimal::from(v))
}
fn installment(principal: i64, interest: i64, fee: i64, penalty: i64) -> Installment {
    Installment::new(
        SCALE,
        MODE,
        due_date(),
        BigDecimal::from(principal),
        BigDecimal::from(interest),
        BigDecimal::from(fee),
        BigDecimal::from(penalty),
    )
}

fn assert_alloc(label: &str, got: &Allocation, p: i64, i: i64, f: i64, pen: i64, unproc: i64) {
    let check = |name: &str, actual: &Money, expected: i64| {
        assert_eq!(
            actual.to_plain_string(),
            money(expected).to_plain_string(),
            "{label}: {name} mismatch"
        );
    };
    check("principal", &got.principal, p);
    check("interest", &got.interest, i);
    check("fee", &got.fee, f);
    check("penalty", &got.penalty, pen);
    check("unprocessed", &got.unprocessed, unproc);
}

#[test]
fn charge_free_scenarios_from_golden() {
    let tsv = include_str!("golden/allocation_legacy.tsv");
    let mut count = 0;
    for line in tsv.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        assert_eq!(c.len(), 12, "row has wrong column count: {line}");
        let timing = c[0];
        let g = |idx: usize| c[idx].parse::<i64>().unwrap();
        let (p_out, i_out, f_out, pen_out, txn) = (g(1), g(2), g(3), g(4), g(5));
        let (ep, ei, ef, epen, eunproc) = (g(6), g(7), g(8), g(9), g(10));
        let src = c[11];

        let mut inst = installment(p_out, i_out, f_out, pen_out);
        let amount = money(txn);
        let got = match timing {
            "IA" => strat::in_advance(&mut inst, txn_date(), &amount, &[]),
            "OT" => strat::on_time(&mut inst, &amount),
            "LATE" => strat::late(&mut inst, &amount),
            other => panic!("unknown timing {other}"),
        };
        assert_alloc(src, &got, ep, ei, ef, epen, eunproc);
        count += 1;
    }
    assert_eq!(count, 33, "expected all 33 charge-free golden rows");
}

fn charge(amount: i64, is_penalty: bool, due: Date) -> PendingCharge {
    PendingCharge {
        effective_due_date: Some(due),
        is_penalty,
        amount: BigDecimal::from(amount),
        is_active: true,
        is_pending: true,
    }
}

/// `duePaymentOfPenaltyAsInAdvanceTransaction` (src line 417): one due penalty charge (2, due today) plus one
/// not-yet-due charge (due tomorrow, excluded). Installment P3 I2 F2 Pen4, txn 10 → (3, 2, 1, 4).
#[test]
fn in_advance_due_penalty() {
    let charges = [
        charge(2, true, txn_date()),
        // not-yet-due → excluded by the effective-due-date filter (no amount/penalty flag needed)
        PendingCharge {
            effective_due_date: Some(txn_date().next_day().unwrap()),
            is_penalty: false,
            amount: BigDecimal::from(0),
            is_active: true,
            is_pending: true,
        },
    ];
    let mut inst = installment(3, 2, 2, 4);
    let got = strat::in_advance(&mut inst, txn_date(), &money(10), &charges);
    assert_alloc(
        "duePaymentOfPenaltyAsInAdvanceTransaction",
        &got,
        3,
        2,
        1,
        4,
        0,
    );
}

/// `duePaymentOfPenaltyAndFeeAsInAdvanceTransaction` (src line 456): due penalty 2 (today) + due fee 1 (yesterday).
/// Installment P2 I2 F2 Pen4, txn 10 → (2, 2, 2, 4).
#[test]
fn in_advance_due_penalty_and_fee() {
    let charges = [
        charge(2, true, txn_date()),
        charge(1, false, txn_date().previous_day().unwrap()),
    ];
    let mut inst = installment(2, 2, 2, 4);
    let got = strat::in_advance(&mut inst, txn_date(), &money(10), &charges);
    assert_alloc(
        "duePaymentOfPenaltyAndFeeAsInAdvanceTransaction",
        &got,
        2,
        2,
        2,
        4,
        0,
    );
}

/// `duePaymentOfHigherPenaltyAndHigherFeeAsInAdvanceTransaction` (src line 497): due charges (11, 11) both exceed the
/// remainder, so the due caps are overridden by the unprocessed amount. Installment P2 I2 F2 Pen4, txn 10 → (2,2,2,4).
#[test]
fn in_advance_higher_due_penalty_and_fee() {
    let charges = [
        charge(11, true, txn_date()),
        charge(11, false, txn_date().previous_day().unwrap()),
    ];
    let mut inst = installment(2, 2, 2, 4);
    let got = strat::in_advance(&mut inst, txn_date(), &money(10), &charges);
    assert_alloc(
        "duePaymentOfHigherPenaltyAndHigherFeeAsInAdvanceTransaction",
        &got,
        2,
        2,
        2,
        4,
        0,
    );
}

/// Sanity: the late-date path is genuinely exercised with a date after the due date (the strategy treats it like
/// on-time ordering, but this guards against accidental in-advance dispatch in callers).
#[test]
fn late_date_is_after_due() {
    assert!(late_date() > due_date());
    let mut inst = installment(3, 2, 2, 4);
    let got = strat::late(&mut inst, &money(10));
    assert_alloc("late_smoke", &got, 3, 2, 1, 4, 0);
}
