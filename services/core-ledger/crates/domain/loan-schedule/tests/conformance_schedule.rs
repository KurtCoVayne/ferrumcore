//! Conformance: regenerate the two captured schedule scenarios and assert EXACT match to the golden produced by the
//! real `ProgressiveEMICalculator` (`plan/harness/golden/schedule_corpus.tsv`, GREEN-verified) — EMI, principal,
//! interest, and outstanding balance per period.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::RoundingMode;
use fc_loan_schedule::{DayCount, MathCtx, PeriodBoundary, ScheduleParams, generate};
use fc_money::decimal::to_plain_string;
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::from_str(s).unwrap()
}

/// First-of-month date `k` months after 2024-01.
fn first_of(k: i32) -> Date {
    let total = k;
    let year = 2024 + total.div_euclid(12);
    let month = u8::try_from(total.rem_euclid(12) + 1).unwrap();
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), 1).unwrap()
}

fn monthly(n: usize) -> Vec<PeriodBoundary> {
    (0..n)
        .map(|k| {
            let k = i32::try_from(k).unwrap();
            PeriodBoundary {
                from: first_of(k),
                due: first_of(k + 1),
            }
        })
        .collect()
}

fn run(
    scenario: &str,
    rate: &str,
    days_in_year: i64,
    days_in_month_30: bool,
    months: usize,
) -> Vec<String> {
    let params = ScheduleParams {
        disbursed: bd("100"),
        annual_rate_pct: bd(rate),
        day_count: DayCount {
            days_in_year,
            days_in_month_30,
        },
        currency_scale: 2,
        mc: MathCtx::new(12, RoundingMode::HalfEven),
    };
    generate(&params, &monthly(months))
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "{scenario}\t{}\t{}\t{}\t{}\t{}",
                i + 1,
                to_plain_string(&p.emi),
                to_plain_string(&p.principal),
                to_plain_string(&p.interest),
                to_plain_string(&p.outstanding),
            )
        })
        .collect()
}

#[test]
fn schedule_matches_real_emi_calculator_golden() {
    let mut got: Vec<String> = Vec::new();
    got.extend(run(
        "emiAdj_100_15.678_365_ACTUAL_6m",
        "15.678",
        365,
        false,
        6,
    ));
    got.extend(run("disb_100_9.4822_360_30_12m", "9.4822", 360, true, 12));

    let golden: Vec<String> = include_str!("golden/schedule_corpus.tsv")
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(std::string::ToString::to_string)
        .collect();

    assert_eq!(golden.len(), 18, "unexpected golden size");
    let mut mismatches = Vec::new();
    for (g, h) in golden.iter().zip(got.iter()) {
        if g != h {
            mismatches.push(format!("golden: {g}\n   got: {h}"));
        }
    }
    assert_eq!(
        golden.len(),
        got.len(),
        "row count differs: {} vs {}",
        golden.len(),
        got.len()
    );
    assert!(
        mismatches.is_empty(),
        "{} / {} schedule rows diverge from the real ProgressiveEMICalculator:\n{}",
        mismatches.len(),
        golden.len(),
        mismatches.join("\n")
    );
}
