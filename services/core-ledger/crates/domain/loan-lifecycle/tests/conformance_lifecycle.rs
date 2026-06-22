//! Conformance: replay the loan lifecycle transition golden (transcribed from the original, GREEN-verified) and assert
//! fc-loan-lifecycle reproduces every (event, from, financial-ctx) → next-status decision (BI-1 / invariant I2).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use fc_loan_lifecycle::{Ctx, LoanEvent, LoanStatus, next_status};

fn flag(s: &str) -> bool {
    s == "t"
}
fn parse_from(s: &str) -> Option<LoanStatus> {
    if s == "NONE" {
        None
    } else {
        Some(LoanStatus::from_name(s).expect("status name"))
    }
}
fn fmt(o: Option<LoanStatus>) -> String {
    o.map_or_else(|| "NONE".to_string(), |s| format!("{s:?}"))
}
fn expected_name(s: &str) -> String {
    // map the golden's Java constant name to the Rust Debug name for comparison
    if s == "NONE" {
        "NONE".to_string()
    } else {
        format!("{:?}", LoanStatus::from_name(s).expect("expected status"))
    }
}

#[test]
fn lifecycle_matches_original_state_machine() {
    let corpus = include_str!("golden/lifecycle_transitions.tsv");
    let (mut total, mut mismatches) = (0u64, Vec::new());
    for line in corpus.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let event = LoanEvent::from_name(c[0]);
        let from = parse_from(c[1]);
        let ctx = Ctx {
            overpaid: flag(c[2]),
            total_overpaid_zero: flag(c[3]),
            outstanding_zero: flag(c[4]),
            repaid_in_full: flag(c[5]),
        };
        let want = expected_name(c[6]);
        let got = fmt(next_status(event, from, ctx));
        total += 1;
        if got != want {
            mismatches.push(format!(
                "{} from={} ctx={ctx:?}: expected={want} got={got}",
                c[0], c[1]
            ));
        }
    }
    assert!(total >= 35, "golden too small: {total}");
    assert!(
        mismatches.is_empty(),
        "{} / {total} transitions diverge from the original:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
