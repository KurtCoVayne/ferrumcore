//! Conformance: for every captured scenario, regenerate the schedule and assert EXACT match to the golden produced by
//! the real `ProgressiveEMICalculator` (`plan/harness/golden/schedule_corpus.tsv`, GREEN-verified). Covers a range of
//! rates (incl. interest-free), terms (3–24 months), principals, and both day-count bases — EMI/principal/interest/
//! outstanding per period.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::RoundingMode;
use fc_loan_schedule::{DayCount, MathCtx, PeriodBoundary, ScheduleParams, generate};
use fc_money::decimal::to_plain_string;
use std::collections::BTreeMap;
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::from_str(s).unwrap()
}

/// First-of-month date `k` months after 2024-01.
fn first_of(k: i32) -> Date {
    let year = 2024 + k.div_euclid(12);
    let month = u8::try_from(k.rem_euclid(12) + 1).unwrap();
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

/// Parse a scenario name: `r<rate>_d<disburse>_y<daysInYear>_m<ACTUAL|30>_n<months>`.
fn parse(name: &str) -> ScheduleParams {
    let p: Vec<&str> = name.split('_').collect();
    let rate = &p[0][1..];
    let disburse = &p[1][1..];
    let diy: i64 = p[2][1..].parse().unwrap();
    let dim = &p[3][1..];
    ScheduleParams {
        disbursed: bd(disburse),
        annual_rate_pct: bd(rate),
        day_count: DayCount {
            days_in_year: diy,
            days_in_month_30: dim == "30",
        },
        currency_scale: 2,
        mc: MathCtx::new(12, RoundingMode::HalfEven),
    }
}
fn months_of(name: &str) -> usize {
    name.split('_').next_back().unwrap()[1..].parse().unwrap()
}

#[test]
fn schedule_matches_real_emi_calculator_golden() {
    // golden keyed by "scenario\tperiod" -> full line, preserving scenario order
    let mut golden: BTreeMap<String, String> = BTreeMap::new();
    let mut scenarios: Vec<String> = Vec::new();
    for line in include_str!("golden/schedule_corpus.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let key = format!("{}\t{}", c[0], c[1]);
        if !scenarios.contains(&c[0].to_string()) {
            scenarios.push(c[0].to_string());
        }
        golden.insert(key, line.to_string());
    }
    assert_eq!(golden.len(), 75, "unexpected golden size: {}", golden.len());

    let mut mismatches = Vec::new();
    let mut checked = 0u64;
    for scenario in &scenarios {
        let params = parse(scenario);
        let rows = generate(&params, &monthly(months_of(scenario)));
        for (i, r) in rows.iter().enumerate() {
            let key = format!("{scenario}\t{}", i + 1);
            let got = format!(
                "{key}\t{}\t{}\t{}\t{}",
                to_plain_string(&r.emi),
                to_plain_string(&r.principal),
                to_plain_string(&r.interest),
                to_plain_string(&r.outstanding),
            );
            checked += 1;
            match golden.get(&key) {
                Some(g) if *g == got => {}
                Some(g) => mismatches.push(format!("golden: {g}\n   got: {got}")),
                None => mismatches.push(format!("missing golden for {key}")),
            }
        }
    }
    assert_eq!(
        checked as usize,
        golden.len(),
        "row count differs: {checked} vs {}",
        golden.len()
    );
    assert!(
        mismatches.is_empty(),
        "{} / {} schedule rows diverge from the real ProgressiveEMICalculator:\n{}",
        mismatches.len(),
        golden.len(),
        mismatches
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
