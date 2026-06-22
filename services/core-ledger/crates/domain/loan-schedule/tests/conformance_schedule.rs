//! Conformance: for every captured scenario, regenerate the schedule and assert EXACT match to the golden produced by
//! the real `ProgressiveEMICalculator` (`plan/harness/golden/schedule_corpus.tsv`, GREEN-verified). Covers the full
//! schedule-generation config space: declining + flat interest; monthly/weekly/daily frequency; repay-every; daily +
//! same-as-repayment calc; both day counts; principal grace; varied rates/terms/principals.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::RoundingMode;
use fc_domain_shared::{InterestCalculationPeriodMethod, InterestMethod, PeriodFrequencyType};
use fc_loan_schedule::{DayCount, MathCtx, PeriodBoundary, ScheduleParams, generate};
use fc_money::decimal::to_plain_string;
use std::collections::BTreeMap;
use std::str::FromStr;
use time::{Date, Duration, Month};

fn bd(s: &str) -> bigdecimal::BigDecimal {
    bigdecimal::BigDecimal::from_str(s).unwrap()
}
fn base() -> Date {
    Date::from_calendar_date(2024, Month::January, 1).unwrap()
}
fn first_of_month(k: i64) -> Date {
    let year = 2024 + i32::try_from(k.div_euclid(12)).unwrap();
    let month = u8::try_from(k.rem_euclid(12) + 1).unwrap();
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), 1).unwrap()
}

fn boundaries(freq: PeriodFrequencyType, every: i64, n: usize) -> Vec<PeriodBoundary> {
    (0..n)
        .map(|i| {
            let i = i as i64;
            let (from, due) = match freq {
                PeriodFrequencyType::Months => {
                    (first_of_month(i * every), first_of_month((i + 1) * every))
                }
                PeriodFrequencyType::Weeks => (
                    base() + Duration::days(7 * every * i),
                    base() + Duration::days(7 * every * (i + 1)),
                ),
                _ => (
                    base() + Duration::days(every * i),
                    base() + Duration::days(every * (i + 1)),
                ),
            };
            PeriodBoundary { from, due }
        })
        .collect()
}

struct Parsed {
    params: ScheduleParams,
    freq: PeriodFrequencyType,
    every: i64,
    n: usize,
}

fn parse(name: &str) -> Parsed {
    let p: Vec<&str> = name.split('_').collect();
    let method = match p[0] {
        "DECL" => InterestMethod::DecliningBalance,
        "FLAT" => InterestMethod::Flat,
        x => panic!("method {x}"),
    };
    let calc = match p[1] {
        "DLY" => InterestCalculationPeriodMethod::Daily,
        "SAME" => InterestCalculationPeriodMethod::SameAsRepaymentPeriod,
        x => panic!("calc {x}"),
    };
    let freq = match &p[2][..2] {
        "MO" => PeriodFrequencyType::Months,
        "WK" => PeriodFrequencyType::Weeks,
        "DY" => PeriodFrequencyType::Days,
        x => panic!("freq {x}"),
    };
    let every: i64 = p[2][3..].parse().unwrap();
    let diy: i64 = p[5][1..].parse().unwrap();
    let dim30 = &p[6][1..] == "30";
    let grace: usize = p[7][2..].parse().unwrap();
    let n: usize = p[8][1..].parse().unwrap();
    Parsed {
        params: ScheduleParams {
            disbursed: bd(&p[4][1..]),
            annual_rate_pct: bd(&p[3][1..]),
            day_count: DayCount {
                days_in_year: diy,
                days_in_month_30: dim30,
            },
            currency_scale: 2,
            mc: MathCtx::new(12, RoundingMode::HalfEven),
            interest_method: method,
            calc_method: calc,
            frequency: freq,
            repay_every: every,
            grace_on_principal: grace,
        },
        freq,
        every,
        n,
    }
}

#[test]
fn schedule_matches_real_emi_calculator_golden() {
    let mut golden: BTreeMap<String, String> = BTreeMap::new();
    let mut scenarios: Vec<String> = Vec::new();
    for line in include_str!("golden/schedule_corpus.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if !scenarios.contains(&c[0].to_string()) {
            scenarios.push(c[0].to_string());
        }
        golden.insert(format!("{}\t{}", c[0], c[1]), line.to_string());
    }
    assert!(
        golden.len() >= 140,
        "unexpected golden size: {}",
        golden.len()
    );

    let mut mismatches = Vec::new();
    let mut checked = 0u64;
    for scenario in &scenarios {
        let parsed = parse(scenario);
        let rows = generate(
            &parsed.params,
            &boundaries(parsed.freq, parsed.every, parsed.n),
        );
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
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
