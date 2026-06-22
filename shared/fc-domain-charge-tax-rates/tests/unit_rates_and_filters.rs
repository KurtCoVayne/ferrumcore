//! Unit tests: tax date-range filtering & None-percentage skip, percentage-based predicate, and floating-rate
//! selection (`effective_rate_period` fixed point-in-time; `fetch_interest_rates` faithful to `fetchInterestRates`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_domain_charge_tax_rates::{
    ChargeCalculationType, FloatingRatePeriod, TaxComponent, effective_rate_period,
    fetch_interest_rates, split_tax,
};
use std::str::FromStr;
use time::{Date, Month};

fn d(y: i32, m: Month, day: u8) -> Date {
    Date::from_calendar_date(y, m, day).unwrap()
}
fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}

#[test]
fn percentage_based_predicate() {
    assert!(!ChargeCalculationType::Invalid.is_percentage_based());
    assert!(!ChargeCalculationType::Flat.is_percentage_based());
    assert!(ChargeCalculationType::PercentOfAmount.is_percentage_based());
    assert!(ChargeCalculationType::PercentOfAmountAndInterest.is_percentage_based());
    assert!(ChargeCalculationType::PercentOfInterest.is_percentage_based());
    assert!(ChargeCalculationType::PercentOfDisbursementAmount.is_percentage_based());
}

#[test]
fn tax_mapping_inclusive_date_range_and_skip() {
    let comp = |start, end, pct: Option<&str>| TaxComponent {
        percentage: pct.map(bd),
        start,
        end,
    };
    let comps = vec![
        comp(
            Some(d(2024, Month::January, 1)),
            Some(d(2024, Month::December, 31)),
            Some("10"),
        ), // in range
        comp(Some(d(2025, Month::January, 1)), None, Some("5")), // future
        comp(None, None, None),                                  // null pct -> skipped
    ];
    let on = d(2024, Month::June, 15);
    let split = split_tax(&bd("1000.00"), &comps, on, 2, RoundingMode::HalfEven);
    // Only component 0 applies: inclusive range covers it, future excluded, None-percentage skipped.
    assert_eq!(split.len(), 1);
    assert_eq!(split[0].0, 0);

    // Inclusive boundaries.
    assert!(comps[0].occurs_on(d(2024, Month::January, 1)));
    assert!(comps[0].occurs_on(d(2024, Month::December, 31)));
    assert!(!comps[0].occurs_on(d(2023, Month::December, 31)));
    assert!(!comps[0].occurs_on(d(2025, Month::January, 1)));
}

fn periods() -> Vec<FloatingRatePeriod> {
    // Ordered by fromDate as the original @OrderBy requires.
    vec![
        FloatingRatePeriod {
            from: d(2024, Month::January, 1),
            rate: bd("5.0"),
            is_differential_to_base_lending_rate: false,
            is_active: true,
        },
        FloatingRatePeriod {
            from: d(2024, Month::April, 1),
            rate: bd("6.0"),
            is_differential_to_base_lending_rate: false,
            is_active: true,
        },
        FloatingRatePeriod {
            from: d(2024, Month::July, 1),
            rate: bd("7.0"),
            is_differential_to_base_lending_rate: false,
            is_active: false,
        }, // inactive
        FloatingRatePeriod {
            from: d(2024, Month::October, 1),
            rate: bd("8.0"),
            is_differential_to_base_lending_rate: false,
            is_active: true,
        },
    ]
}

#[test]
fn fixed_point_in_time_selection() {
    let p = periods();
    // Latest active from <= date.
    assert_eq!(
        effective_rate_period(&p, d(2024, Month::May, 1))
            .unwrap()
            .rate,
        bd("6.0")
    );
    // Inactive July period is skipped -> falls back to April's 6.0.
    assert_eq!(
        effective_rate_period(&p, d(2024, Month::August, 1))
            .unwrap()
            .rate,
        bd("6.0")
    );
    assert_eq!(
        effective_rate_period(&p, d(2024, Month::November, 1))
            .unwrap()
            .rate,
        bd("8.0")
    );
    // Before all periods -> earliest active.
    assert_eq!(
        effective_rate_period(&p, d(2023, Month::January, 1))
            .unwrap()
            .rate,
        bd("5.0")
    );
}

#[test]
fn floating_returns_active_at_start_then_all_later() {
    let p = periods();
    // start mid-Jan: active-at-start is Jan(5.0); later active periods Apr(6.0), Oct(8.0); inactive Jul excluded.
    let rates: Vec<_> = fetch_interest_rates(&p, d(2024, Month::January, 15), true)
        .iter()
        .map(|x| x.rate.clone())
        .collect();
    assert_eq!(rates, vec![bd("5.0"), bd("6.0"), bd("8.0")]);
}

#[test]
fn fixed_via_fetch_returns_single_active_at_start() {
    let p = periods();
    let rates: Vec<_> = fetch_interest_rates(&p, d(2024, Month::May, 1), false)
        .iter()
        .map(|x| x.rate.clone())
        .collect();
    assert_eq!(rates, vec![bd("6.0")]); // single period active at start
}

#[test]
fn start_after_all_periods_falls_back_to_last_active() {
    let p = periods();
    let rates: Vec<_> = fetch_interest_rates(&p, d(2025, Month::January, 1), true)
        .iter()
        .map(|x| x.rate.clone())
        .collect();
    assert_eq!(rates, vec![bd("8.0")]); // last active period
}
