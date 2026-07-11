//! The pre-replay validation boundary: coherent journals validate clean; incoherent input the kernel would silently
//! clamp (Fineract parity) is surfaced as explicit, typed issues at the edge.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use fc_loan_journal::{LoanJournal, LoanOp, ScheduleInit, ValidationIssue};

fn periods() -> Vec<(String, String)> {
    vec![
        ("2024-01-01".to_string(), "2024-02-01".to_string()),
        ("2024-02-01".to_string(), "2024-03-01".to_string()),
    ]
}

fn valid_init() -> ScheduleInit {
    ScheduleInit::Declining {
        periods: periods(),
        principal: "100".to_string(),
        rate_pct: "7.0".to_string(),
        repay_every: 1,
        scale: 2,
    }
}

/// A coherent journal validates clean.
#[test]
fn coherent_journal_has_no_issues() {
    let journal = LoanJournal {
        init: valid_init(),
        ops: vec![LoanOp::PayPrincipal {
            from: "2024-01-01".to_string(),
            due: "2024-02-01".to_string(),
            pay_date: "2024-01-15".to_string(),
            amount: "10".to_string(),
        }],
    };
    assert!(journal.validate().is_empty());
}

/// Empty schedule, non-positive principal, and negative rate are each surfaced.
#[test]
fn incoherent_init_is_flagged() {
    let journal = LoanJournal {
        init: ScheduleInit::Declining {
            periods: vec![],
            principal: "0".to_string(),
            rate_pct: "-1".to_string(),
            repay_every: 1,
            scale: 9,
        },
        ops: vec![],
    };
    let issues = journal.validate();
    assert!(issues.contains(&ValidationIssue::NoPeriods));
    assert!(issues.contains(&ValidationIssue::NonPositivePrincipal));
    assert!(issues.contains(&ValidationIssue::NegativeRate));
    assert!(issues.contains(&ValidationIssue::ScaleOutOfRange(9)));
}

/// A period whose due is not after its from, and out-of-order periods, are flagged by index.
#[test]
fn bad_period_bounds_are_flagged() {
    let journal = LoanJournal {
        init: ScheduleInit::Declining {
            periods: vec![
                ("2024-03-01".to_string(), "2024-03-01".to_string()), // due == from
                ("2024-01-01".to_string(), "2024-02-01".to_string()), // out of order
            ],
            principal: "100".to_string(),
            rate_pct: "7".to_string(),
            repay_every: 1,
            scale: 2,
        },
        ops: vec![],
    };
    let issues = journal.validate();
    assert!(issues.contains(&ValidationIssue::NonPositivePeriod(0)));
    assert!(issues.contains(&ValidationIssue::PeriodsOutOfOrder(1)));
}

/// A negative op amount and a zero-installment re-age (the `.max(1)` silent-clamp case) are surfaced by op index.
#[test]
fn incoherent_ops_are_flagged() {
    let journal = LoanJournal {
        init: valid_init(),
        ops: vec![
            LoanOp::CreditPrincipal {
                date: "2024-01-15".to_string(),
                amount: "-5".to_string(),
            },
            LoanOp::ReAge {
                transaction_date: "2024-02-01".to_string(),
                re_age_start: "2024-02-05".to_string(),
                installments: 0,
                frequency_months: 1,
                full_interest: true,
            },
            LoanOp::AddRepaymentPeriods {
                submitted_on: "2024-02-01".to_string(),
                count: 0,
            },
        ],
    };
    let issues = journal.validate();
    assert!(issues.contains(&ValidationIssue::NegativeAmount(0)));
    assert!(issues.contains(&ValidationIssue::ZeroReAgeInstallments(1)));
    assert!(issues.contains(&ValidationIssue::ZeroRepaymentPeriodCount(2)));
}

/// Unparseable fields are surfaced (not silently dropped), with a human-readable location.
#[test]
fn unparseable_fields_are_flagged() {
    let journal = LoanJournal {
        init: ScheduleInit::Declining {
            periods: vec![("2024-99-99".to_string(), "2024-02-01".to_string())],
            principal: "abc".to_string(),
            rate_pct: "7".to_string(),
            repay_every: 1,
            scale: 2,
        },
        ops: vec![LoanOp::Disburse {
            date: "2024-01-01".to_string(),
            amount: "nope".to_string(),
        }],
    };
    let issues = journal.validate();
    assert!(
        issues
            .iter()
            .any(|i| matches!(i, ValidationIssue::Unparseable(s) if s.contains("principal"))),
        "principal parse error surfaced"
    );
    assert!(
        issues
            .iter()
            .any(|i| matches!(i, ValidationIssue::Unparseable(s) if s.contains("op 0 amount"))),
        "op amount parse error surfaced"
    );
}
