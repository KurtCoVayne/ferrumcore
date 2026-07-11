//! Conformance: the persisted operation log replays to a model **byte-identical** to the live-operated one
//! (ADR-0026's enforcing invariant), and survives a serde JSON round-trip. The two sequences are the golden ones the
//! ADR names: the `test_payoff_on0215` payment sequence and the FULL_INTEREST re-age.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::recalc::{
    DayCount, Model, new_declining_balance, new_declining_balance_with_day_count,
};
use fc_loan_journal::{LoanJournal, LoanOp, ScheduleInit};
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn d(y: i32, m: Month, day: u8) -> Date {
    Date::from_calendar_date(y, m, day).unwrap()
}

fn six_monthly() -> Vec<(Date, Date)> {
    vec![
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ]
}

fn six_monthly_strings() -> Vec<(String, String)> {
    six_monthly()
        .iter()
        .map(|(f, t)| {
            (
                fc_loan_journal::format_date(*f),
                fc_loan_journal::format_date(*t),
            )
        })
        .collect()
}

/// Assert two models are byte-identical across every observable per-period quantity — the reconstruction contract.
fn assert_models_equal(a: &Model, b: &Model) {
    assert_eq!(a.periods.len(), b.periods.len(), "period count");
    for i in 0..a.periods.len() {
        assert_eq!(a.periods[i].from, b.periods[i].from, "period {i} from");
        assert_eq!(a.periods[i].due, b.periods[i].due, "period {i} due");
        assert_eq!(
            a.period_emi(i).to_plain_string(),
            b.period_emi(i).to_plain_string(),
            "period {i} emi"
        );
        assert_eq!(
            a.period_due_principal(i).to_plain_string(),
            b.period_due_principal(i).to_plain_string(),
            "period {i} due principal"
        );
        assert_eq!(
            a.period_due_interest(i).to_plain_string(),
            b.period_due_interest(i).to_plain_string(),
            "period {i} due interest"
        );
        assert_eq!(
            a.period_outstanding(i).to_plain_string(),
            b.period_outstanding(i).to_plain_string(),
            "period {i} outstanding"
        );
        assert_eq!(
            a.period_fully_paid(i),
            b.period_fully_paid(i),
            "period {i} fully paid"
        );
    }
}

/// Payoff sequence (100 @ 7% DAYS_360/30, pay period 1 early on Feb 15): the journal replays byte-identically to the
/// live sequence, and a serde JSON round-trip of the journal is lossless (and float-free).
#[test]
fn payoff_journal_replays_to_live_model() {
    // Live: the exact `payoff_two_payments_with_balance_correction` sequence.
    let mut live = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    live.pay_principal(
        d(2024, Month::February, 1),
        d(2024, Month::March, 1),
        d(2024, Month::February, 15),
        &fc_money::Money::of(2, RoundingMode::HalfEven, &bd("16.77")),
    );
    live.pay_interest(
        d(2024, Month::February, 1),
        d(2024, Month::March, 1),
        d(2024, Month::February, 15),
        &fc_money::Money::of(2, RoundingMode::HalfEven, &bd("0.24")),
    );

    let journal = LoanJournal {
        init: ScheduleInit::Declining {
            periods: six_monthly_strings(),
            principal: "100".to_string(),
            rate_pct: "7.0".to_string(),
            repay_every: 1,
            scale: 2,
        },
        ops: vec![
            LoanOp::PayPrincipal {
                from: "2024-02-01".to_string(),
                due: "2024-03-01".to_string(),
                pay_date: "2024-02-15".to_string(),
                amount: "16.77".to_string(),
            },
            LoanOp::PayInterest {
                from: "2024-02-01".to_string(),
                due: "2024-03-01".to_string(),
                pay_date: "2024-02-15".to_string(),
                amount: "0.24".to_string(),
            },
        ],
    };

    // Replay the in-memory journal.
    let replayed = journal.replay().unwrap();
    assert_models_equal(&live, &replayed);

    // Round-trip through JSON (the persisted form) and replay again — still byte-identical.
    let json = serde_json::to_string(&journal).unwrap();
    assert!(
        json.contains("\"16.77\""),
        "amounts persist as exact-decimal strings"
    );
    assert!(
        !json.contains("16.769") && !json.contains("16.77000"),
        "no float artifacts"
    );
    let decoded: LoanJournal = serde_json::from_str(&json).unwrap();
    assert_eq!(
        decoded, journal,
        "journal survives JSON round-trip losslessly"
    );
    assert_models_equal(&live, &decoded.replay().unwrap());
}

/// FULL_INTEREST re-age (100 @ 15.678% DAYS_365, re-age Apr 15 → 6 monthly from Apr 20): the journal replays
/// byte-identically, exercising the day-count init variant and the re-age op.
#[test]
fn re_age_journal_replays_to_live_model() {
    let mut live = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );
    live.re_age_equal_amortization(
        d(2024, Month::April, 15),
        d(2024, Month::April, 20),
        6,
        1,
        true,
    );

    let journal = LoanJournal {
        init: ScheduleInit::DecliningActualFixedYear {
            periods: six_monthly_strings(),
            principal: "100".to_string(),
            rate_pct: "15.678".to_string(),
            days_in_year: 365,
            scale: 2,
        },
        ops: vec![LoanOp::ReAge {
            transaction_date: "2024-04-15".to_string(),
            re_age_start: "2024-04-20".to_string(),
            installments: 6,
            frequency_months: 1,
            full_interest: true,
        }],
    };

    let json = serde_json::to_string(&journal).unwrap();
    let decoded: LoanJournal = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, journal);
    assert_models_equal(&live, &decoded.replay().unwrap());
}

/// A malformed persisted log surfaces a typed error rather than panicking (deny-panic at the persistence boundary).
#[test]
fn malformed_log_is_a_typed_error() {
    let journal = LoanJournal {
        init: ScheduleInit::Declining {
            periods: vec![("2024-13-99".to_string(), "2024-02-01".to_string())],
            principal: "100".to_string(),
            rate_pct: "7.0".to_string(),
            repay_every: 1,
            scale: 2,
        },
        ops: vec![],
    };
    assert!(journal.replay().is_err());
}
