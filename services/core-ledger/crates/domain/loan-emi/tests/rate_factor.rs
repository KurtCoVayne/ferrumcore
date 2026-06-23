//! Conformance for the rate-factor / `fnValue` primitives, replaying the VERIFIED engine unit tests
//! `ProgressiveEMICalculatorTest.test_rateFactorByRepaymentEveryMonthMethod_*` and `test_fnValueFunction_*`
//! (golden baseline re-verified on JDK 21). Inputs from that test: interest rate 0.094822, `MathContext(12, HALF_EVEN)`,
//! six monthly periods from 2024-01-01.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::{fn_value, rate_factor_by_repayment_every_month};
use fc_loan_schedule::MathCtx;
use fc_money::decimal::to_plain_string;
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn first_of(month: Month) -> Date {
    Date::from_calendar_date(2024, month, 1).unwrap()
}

/// (fromDate, dueDate) for the six monthly periods from 2024-01-01.
fn periods() -> Vec<(Date, Date)> {
    let months = [
        Month::January,
        Month::February,
        Month::March,
        Month::April,
        Month::May,
        Month::June,
        Month::July,
    ];
    (0..6)
        .map(|i| (first_of(months[i]), first_of(months[i + 1])))
        .collect()
}

fn days(from: Date, due: Date) -> BigDecimal {
    BigDecimal::from((due - from).whole_days())
}

#[test]
fn rate_factor_days365_actual_month() {
    let mc = MathCtx::new(12, RoundingMode::HalfEven);
    let rate = bd("0.094822");
    let expected = [
        "0.008053375342",
        "0.007533802740",
        "0.008053375342",
        "0.007793589041",
        "0.008053375342",
        "0.007793589041",
    ];
    for (i, (from, due)) in periods().into_iter().enumerate() {
        let days_in_period = days(from, due); // monthly first-of-month → days-in-period == days-in-month
        let rf = rate_factor_by_repayment_every_month(
            &mc,
            &rate,
            &BigDecimal::from(1),
            &days_in_period,
            &BigDecimal::from(365),
            &days_in_period,
            &days_in_period,
        );
        assert_eq!(to_plain_string(&rf), expected[i], "period {}", i + 1);
    }
}

#[test]
fn rate_factor_actual_year_actual_month() {
    let mc = MathCtx::new(12, RoundingMode::HalfEven);
    let rate = bd("0.094822");
    // 2024 is a leap year → actual days in year = 366.
    let expected = [
        "0.008031371585",
        "0.007513218579",
        "0.008031371585",
        "0.007772295082",
        "0.008031371585",
        "0.007772295082",
    ];
    for (i, (from, due)) in periods().into_iter().enumerate() {
        let days_in_period = days(from, due);
        let rf = rate_factor_by_repayment_every_month(
            &mc,
            &rate,
            &BigDecimal::from(1),
            &days_in_period,
            &BigDecimal::from(366),
            &days_in_period,
            &days_in_period,
        );
        assert_eq!(to_plain_string(&rf), expected[i], "period {}", i + 1);
    }
}

#[test]
fn fn_value_sequence_days365() {
    let mc = MathCtx::new(12, RoundingMode::HalfEven);
    let rate = bd("0.094822");
    let expected = [
        "1.00000000000",
        "2.00753380274",
        "3.02370122596",
        "4.04726671069",
        "5.07986086861",
        "6.11945121660",
    ];
    let mut previous = BigDecimal::from(0);
    for (i, (from, due)) in periods().into_iter().enumerate() {
        let days_in_period = days(from, due);
        let rate_factor = rate_factor_by_repayment_every_month(
            &mc,
            &rate,
            &BigDecimal::from(1),
            &days_in_period,
            &BigDecimal::from(365),
            &days_in_period,
            &days_in_period,
        );
        // The original test rounds `rateFactor + 1` to the MathContext precision (`.add(ONE, mc)`).
        let rate_factor_plus_one = mc.add(&rate_factor, &BigDecimal::from(1));
        let current = fn_value(&mc, &previous, &rate_factor_plus_one);
        assert_eq!(
            to_plain_string(&current),
            expected[i],
            "fn period {}",
            i + 1
        );
        previous = current;
    }
}
