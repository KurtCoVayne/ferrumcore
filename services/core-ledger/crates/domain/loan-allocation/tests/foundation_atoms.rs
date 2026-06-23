//! Characterization of the shared foundation branches that every legacy strategy reuses verbatim: the waiver /
//! charge-payment dispatch (`nonrepayment`) and the refund ordering (`common_refund` / `unpay_*` atoms). Expectations
//! are hand-traced from the original `Abstract/…LoanRepaymentScheduleTransactionProcessor` + `LoanRepaymentSchedule
//! Installment` semantics (the same atoms already golden-verified by `conformance_allocation.rs`). Currency USD/2,
//! HALF_EVEN.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::strategy::due_pen_int_pri_fee::DuePenIntPriFee;
use fc_loan_allocation::strategy::{allocate_on_time, allocate_refund};
use fc_loan_allocation::{Allocation, Installment, Txn, TxnKind};
use fc_money::Money;
use time::{Date, Month};

const SCALE: i64 = 2;
const MODE: RoundingMode = RoundingMode::HalfEven;
const S: DuePenIntPriFee = DuePenIntPriFee;

fn date() -> Date {
    Date::from_calendar_date(2023, Month::July, 31).unwrap()
}
fn money(v: i64) -> Money {
    Money::of(SCALE, MODE, &BigDecimal::from(v))
}
fn installment(p: i64, i: i64, f: i64, pen: i64) -> Installment {
    Installment::new(
        SCALE,
        MODE,
        date(),
        BigDecimal::from(p),
        BigDecimal::from(i),
        BigDecimal::from(f),
        BigDecimal::from(pen),
    )
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

#[test]
fn charge_payment_penalty_pays_only_penalty() {
    let mut inst = installment(0, 0, 0, 5);
    let txn = Txn {
        amount: money(10),
        date: date(),
        kind: TxnKind::ChargePayment { is_penalty: true },
    };
    let got = allocate_on_time(&S, &mut inst, &txn);
    assert_alloc("charge_payment_penalty", &got, 0, 0, 0, 5, 5);
}

#[test]
fn charge_payment_fee_pays_only_fee() {
    let mut inst = installment(0, 0, 5, 0);
    let txn = Txn {
        amount: money(10),
        date: date(),
        kind: TxnKind::ChargePayment { is_penalty: false },
    };
    let got = allocate_on_time(&S, &mut inst, &txn);
    assert_alloc("charge_payment_fee", &got, 0, 0, 5, 0, 5);
}

#[test]
fn interest_waiver_waives_interest_up_to_amount() {
    let mut inst = installment(0, 5, 0, 0);
    let txn = Txn {
        amount: money(10),
        date: date(),
        kind: TxnKind::InterestWaiver,
    };
    let got = allocate_on_time(&S, &mut inst, &txn);
    assert_alloc("interest_waiver", &got, 0, 5, 0, 0, 5);
}

#[test]
fn charges_waiver_waives_penalty_then_fee_portions() {
    // waive penalty portion 5 (of 5 outstanding) and fee portion 3 (of 3 outstanding); amount 100 → unprocessed 92.
    let mut inst = installment(0, 0, 3, 5);
    let txn = Txn {
        amount: money(100),
        date: date(),
        kind: TxnKind::ChargesWaiver {
            penalty_portion: BigDecimal::from(5),
            fee_portion: BigDecimal::from(3),
        },
    };
    let got = allocate_on_time(&S, &mut inst, &txn);
    assert_alloc("charges_waiver", &got, 0, 0, 3, 5, 92);
}

#[test]
fn refund_reverses_principal_first_then_interest() {
    // Pay on-time 8 against P5 I3 → principal 5, interest 3 completed. Refund 7 reverses principal 5 then interest 2.
    let mut inst = installment(5, 3, 0, 0);
    let paid = allocate_on_time(&S, &mut inst, &Txn::repayment(date(), money(8)));
    assert_alloc("refund_setup_pay", &paid, 5, 3, 0, 0, 0);

    let got = allocate_refund(&S, &mut inst, &money(7));
    assert_alloc("refund", &got, 5, 2, 0, 0, 0);
}

#[test]
fn refund_partial_principal_only() {
    let mut inst = installment(5, 0, 0, 0);
    let _ = allocate_on_time(&S, &mut inst, &Txn::repayment(date(), money(5)));
    let got = allocate_refund(&S, &mut inst, &money(4));
    assert_alloc("refund_partial_principal", &got, 4, 0, 0, 0, 0);
}
