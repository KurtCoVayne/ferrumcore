//! Conformance for the declining-balance schedule model, replaying the VERIFIED engine test
//! `ProgressiveEMICalculatorTest.test_emi_calculator_performance`: 100 disbursed over 12 monthly periods at 7% nominal,
//! DAYS_360 / DAYS_30. Asserts: 12 periods, loan term 366 days, EMI 8.65 on every non-final period, every period has
//! positive due principal and due interest, and the final period's outstanding balance is zero.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_emi::{
    DayCountKind, generate_declining_balance, generate_declining_balance_grace, generate_flat,
};
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn first_of(year: i32, month: Month) -> Date {
    Date::from_calendar_date(year, month, 1).unwrap()
}
fn d(year: i32, month: Month, day: u8) -> Date {
    Date::from_calendar_date(year, month, day).unwrap()
}

/// `checkPeriod(model, idx, emi, dueInterest, duePrincipal, outstanding)` from the original test.
fn check_period(
    model: &fc_loan_emi::Model,
    idx: usize,
    emi: &str,
    due_interest: &str,
    due_principal: &str,
    outstanding: &str,
) {
    let p = &model.periods[idx];
    assert_eq!(p.emi.to_plain_string(), emi, "period {idx} emi");
    assert_eq!(
        p.due_interest.to_plain_string(),
        due_interest,
        "period {idx} interest"
    );
    assert_eq!(
        p.due_principal.to_plain_string(),
        due_principal,
        "period {idx} principal"
    );
    assert_eq!(
        p.outstanding_balance.to_plain_string(),
        outstanding,
        "period {idx} outstanding"
    );
}

/// `test_360_30_repayment_schedule_disbursement_month_end`: 2450 @ 9.99% nominal, DAYS_360/DAYS_30, six monthly
/// (month-end) periods, **HALF_UP**. Exact per-period golden from the original `checkPeriod` assertions.
#[test]
fn repayment_schedule_360_30_month_end() {
    let periods = [
        (d(2023, Month::October, 31), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 31)),
        (d(2023, Month::December, 31), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("2450"),
        &bd("9.99"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "420.31", "20.40", "399.91", "2050.09");
    check_period(&model, 1, "420.31", "17.07", "403.24", "1646.85");
    check_period(&model, 2, "420.31", "13.71", "406.60", "1240.25");
    check_period(&model, 3, "420.31", "10.33", "409.98", "830.27");
    check_period(&model, 4, "420.31", "6.91", "413.40", "416.87");
    check_period(&model, 5, "420.34", "3.47", "416.87", "0.00");
}

/// `test_360_30_repayment_schedule_disbursement_near_month_end`: 10000 @ 45% nominal, DAYS_360/DAYS_30, six
/// near-month-end periods, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_360_30_near_month_end() {
    let periods = [
        (d(2023, Month::October, 30), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 30)),
        (d(2023, Month::December, 30), d(2024, Month::January, 30)),
        (d(2024, Month::January, 30), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 30)),
        (d(2024, Month::March, 30), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("10000"),
        &bd("45.0"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "1892.12", "375.00", "1517.12", "8482.88");
    check_period(&model, 1, "1892.12", "318.11", "1574.01", "6908.87");
    check_period(&model, 2, "1892.12", "259.08", "1633.04", "5275.83");
    check_period(&model, 3, "1892.12", "197.84", "1694.28", "3581.55");
    check_period(&model, 4, "1892.12", "134.31", "1757.81", "1823.74");
    check_period(&model, 5, "1892.13", "68.39", "1823.74", "0.00");
}

/// `test_360_30_repayment_schedule_disbursement_repay_every_2_months`: 3333 @ 19.99% nominal, DAYS_360/DAYS_30,
/// repay every 2 months, six periods, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_360_30_repay_every_2_months() {
    let periods = [
        (d(2023, Month::October, 29), d(2023, Month::December, 29)),
        (d(2023, Month::December, 29), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::April, 29)),
        (d(2024, Month::April, 29), d(2024, Month::June, 29)),
        (d(2024, Month::June, 29), d(2024, Month::August, 29)),
        (d(2024, Month::August, 29), d(2024, Month::October, 29)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("3333"),
        &bd("19.99"),
        DayCountKind::Days360Days30 { repay_every: 2 },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "622.04", "111.04", "511.00", "2822.00");
    check_period(&model, 1, "622.04", "94.02", "528.02", "2293.98");
    check_period(&model, 2, "622.04", "76.43", "545.61", "1748.37");
    check_period(&model, 3, "622.04", "58.25", "563.79", "1184.58");
    check_period(&model, 4, "622.04", "39.47", "582.57", "602.01");
    check_period(&model, 5, "622.07", "20.06", "602.01", "0.00");
}

/// `test_disbursedAmt100_dayInYears360_daysInMonth30_repayEvery1Month` (interest-period granularity, HALF_EVEN):
/// 100 @ 9.4822%, 360/30, six monthly periods from 2024-01-01. Asserts the per-interest-period rate factor and
/// `getCalculatedDueInterest`, including the zero-length disbursement stub (rp0 interest period 0).
#[test]
fn disbursed_100_360_30_interest_period_granularity() {
    let model = generate_declining_balance(
        &monthly_periods_2024_six(),
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
    );
    // rp0 has two interest periods: the disbursement stub (rate factor 0) then the real one.
    let rp0 = &model.periods[0];
    assert_eq!(
        rp0.interest_periods.len(),
        2,
        "rp0 has stub + real interest period"
    );
    assert_eq!(rp0.interest_periods[0].rate_factor.to_string(), "0");
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[0].calculated_due_interest),
        "0"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[1].rate_factor),
        "0.007901833333"
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&rp0.interest_periods[1].calculated_due_interest),
        "0.790183333301"
    );
    // repayment-period level: emi 17.13, dueInterest 0.79, duePrincipal 16.34, outstanding 83.66.
    check_period(&model, 0, "17.13", "0.79", "16.34", "83.66");
    check_period(&model, 1, "17.13", "0.66", "16.47", "67.19");
    check_period(&model, 5, "17.13", "0.13", "17.00", "0.00");
    // later periods have a single interest period.
    assert_eq!(model.periods[1].interest_periods.len(), 1);
}

/// `test_disbursedAmt1000_NoInterest_repayEvery1Month`: 1000 at 0% over 4 monthly periods (ACTUAL/ACTUAL) → equal
/// principal of 250, zero interest, balance stepping to 0.
#[test]
fn disbursed_1000_no_interest() {
    let periods: Vec<(Date, Date)> = (0..4)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect();
    let model = generate_declining_balance(
        &periods,
        &bd("1000"),
        &bd("0"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "250.00", "0.00", "250.00", "750.00");
    check_period(&model, 1, "250.00", "0.00", "250.00", "500.00");
    check_period(&model, 2, "250.00", "0.00", "250.00", "250.00");
    check_period(&model, 3, "250.00", "0.00", "250.00", "0.00");
    assert_eq!(
        model.periods[0].interest_periods[0].rate_factor.to_string(),
        "0"
    );
}

/// `test_disbursedAmt100_dayInYears364_daysInMonthActual_repayEvery1Week`: 100 @ 9.4822%, DAYS_364 / ACTUAL, six
/// weekly periods, HALF_EVEN. Exact per-period golden incl. the interest-period rate factor for rp0.
#[test]
fn disbursed_100_364_weekly() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 8)),
        (d(2024, Month::January, 8), d(2024, Month::January, 15)),
        (d(2024, Month::January, 15), d(2024, Month::January, 22)),
        (d(2024, Month::January, 22), d(2024, Month::January, 29)),
        (d(2024, Month::January, 29), d(2024, Month::February, 5)),
        (d(2024, Month::February, 5), d(2024, Month::February, 12)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::ActualFixedYear { days_in_year: 364 },
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.periods[0].interest_periods[1].rate_factor),
        "0.001823500000"
    );
    check_period(&model, 0, "16.77", "0.18", "16.59", "83.41");
    check_period(&model, 1, "16.77", "0.15", "16.62", "66.79");
    check_period(&model, 2, "16.77", "0.12", "16.65", "50.14");
    check_period(&model, 3, "16.77", "0.09", "16.68", "33.46");
    check_period(&model, 4, "16.77", "0.06", "16.71", "16.75");
    check_period(&model, 5, "16.78", "0.03", "16.75", "0.00");
}

/// `test_emiAdjustment_newCalculatedEmiNotBetterThanOriginal`: 100 @ 15.678%, DAYS_365 / ACTUAL, six monthly periods,
/// HALF_EVEN. The per-period rate factor varies with the actual month length (Jan 31, Feb 29, …); the plain annuity EMI
/// (17.43) already matches — confirming the `checkAndAdjustEmiIfNeeded` cent-loop is a no-op for this case.
#[test]
fn repayment_schedule_365_actual_monthly() {
    let model = generate_declining_balance(
        &monthly_periods_2024_six(),
        &bd("100"),
        &bd("15.678"),
        DayCountKind::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.periods[0].interest_periods[1].rate_factor),
        "0.013315561644"
    );
    check_period(&model, 0, "17.43", "1.33", "16.10", "83.90");
    check_period(&model, 1, "17.43", "1.05", "16.38", "67.52");
    check_period(&model, 2, "17.43", "0.90", "16.53", "50.99");
    check_period(&model, 3, "17.43", "0.66", "16.77", "34.22");
    check_period(&model, 4, "17.43", "0.46", "16.97", "17.25");
    check_period(&model, 5, "17.47", "0.22", "17.25", "0.00");
}

/// `test_disbursedAmt100_dayInYears360_daysInMonthDoesntMatter_repayEvery15Days`: 100 @ 9.4822%, DAYS_360 / ACTUAL,
/// repay every 15 days, six periods, HALF_EVEN. Exact per-period golden (fits ActualFixedYear{360}).
#[test]
fn disbursed_100_360_15_days() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 16)),
        (d(2024, Month::January, 16), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 15)),
        (d(2024, Month::February, 15), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::March, 16)),
        (d(2024, Month::March, 16), d(2024, Month::March, 31)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::ActualFixedYear { days_in_year: 360 },
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.periods[0].interest_periods[1].rate_factor),
        "0.003950916667"
    );
    check_period(&model, 0, "16.90", "0.40", "16.50", "83.50");
    check_period(&model, 1, "16.90", "0.33", "16.57", "66.93");
    check_period(&model, 2, "16.90", "0.26", "16.64", "50.29");
    check_period(&model, 3, "16.90", "0.20", "16.70", "33.59");
    check_period(&model, 4, "16.90", "0.13", "16.77", "16.82");
    check_period(&model, 5, "16.89", "0.07", "16.82", "0.00");
}

/// `testFlatDaily_1_Month_360_30` (first disbursement block): 750 @ 12% FLAT, DAYS_360/DAYS_30, 4 monthly periods →
/// flat interest 7.50/period on the original principal, EMI 195.00, equal principal 187.50.
#[test]
fn flat_360_30_monthly() {
    let periods: Vec<(Date, Date)> = (0..4)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect();
    let model = generate_flat(
        &periods,
        &bd("750"),
        &bd("12.0"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "195.00", "7.50", "187.50", "562.50");
    check_period(&model, 1, "195.00", "7.50", "187.50", "375.00");
    check_period(&model, 2, "195.00", "7.50", "187.50", "187.50");
    check_period(&model, 3, "195.00", "7.50", "187.50", "0.00");
}

/// `testFlatDaily_1_Month_Actual_Actual` (first disbursement block): 700 @ 12% FLAT, ACTUAL/ACTUAL, 4 monthly periods
/// in 2024 (leap year → daysInYear 366). Flat interest varies with the actual month length (Jan 31 → 7.11, Feb 29 →
/// 6.66, …), EMI 181.94 with the rounding remainder on the last period.
#[test]
fn flat_actual_actual_monthly() {
    let periods: Vec<(Date, Date)> = (0..4)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect();
    let model = generate_flat(
        &periods,
        &bd("700"),
        &bd("12.0"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "181.94", "7.11", "174.83", "525.17");
    check_period(&model, 1, "181.94", "6.66", "175.28", "349.89");
    check_period(&model, 2, "181.94", "7.11", "174.83", "175.06");
    check_period(&model, 3, "181.95", "6.89", "175.06", "0.00");
}

/// `testFlatDaily_1_week_Actual_Actual` (first disbursement block): 600 @ 12% FLAT, ACTUAL/ACTUAL, 4 weekly periods
/// from 2024-01-01 — flat interest 1.38/week on the original principal, EMI 151.38, equal principal 150.00.
#[test]
fn flat_actual_actual_weekly() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 8)),
        (d(2024, Month::January, 8), d(2024, Month::January, 15)),
        (d(2024, Month::January, 15), d(2024, Month::January, 22)),
        (d(2024, Month::January, 22), d(2024, Month::January, 29)),
    ];
    let model = generate_flat(
        &periods,
        &bd("600"),
        &bd("12.0"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "151.38", "1.38", "150.00", "450.00");
    check_period(&model, 1, "151.38", "1.38", "150.00", "300.00");
    check_period(&model, 2, "151.38", "1.38", "150.00", "150.00");
    check_period(&model, 3, "151.38", "1.38", "150.00", "0.00");
}

/// `testFlatDaily_30_Days_Actual_Actual` (first disbursement block): 400 @ 12% FLAT, ACTUAL/ACTUAL, four 30-day periods
/// from 2024-01-01 — flat interest 3.93/period on the original principal, EMI 103.93, equal principal 100.00.
#[test]
fn flat_actual_actual_30_days() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_flat(
        &periods,
        &bd("400"),
        &bd("12.0"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "103.93", "3.93", "100.00", "300.00");
    check_period(&model, 1, "103.93", "3.93", "100.00", "200.00");
    check_period(&model, 2, "103.93", "3.93", "100.00", "100.00");
    check_period(&model, 3, "103.93", "3.93", "100.00", "0.00");
}

/// `test_sameAsRepayment_month_repay_every_1_periods_3` (first disbursement block): 1000 @ 12%, monthly,
/// SAME_AS_REPAYMENT_PERIOD, FLAT method, 3 periods → rate factor 12%/12 = 0.01, flat interest 10/period, EMI 343.33.
#[test]
fn same_as_repayment_month_flat() {
    let periods: Vec<(Date, Date)> = (0..3)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect();
    let model = generate_flat(
        &periods,
        &bd("1000"),
        &bd("12.0"),
        DayCountKind::SameAsRepaymentMonths { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "343.33", "10.00", "333.33", "666.67");
    check_period(&model, 1, "343.33", "10.00", "333.33", "333.34");
    check_period(&model, 2, "343.34", "10.00", "333.34", "0.00");
}

/// `test_principalGraceForProgressiveSchedule`: 100 @ 7%, DAYS_360/30, six monthly periods, 2 principal-grace
/// periods → periods 0-1 interest-only (EMI 0.58, principal 0, balance 100), then annuity over periods 2-5 (EMI 25.37).
#[test]
fn principal_grace_360_30() {
    let model = generate_declining_balance_grace(
        &monthly_periods_2024_six(),
        &bd("100"),
        &bd("7.0"),
        DayCountKind::Days360Days30 { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
        2,
    );
    check_period(&model, 0, "0.58", "0.58", "0.00", "100.00");
    check_period(&model, 1, "0.58", "0.58", "0.00", "100.00");
    check_period(&model, 2, "25.37", "0.58", "24.79", "75.21");
    check_period(&model, 3, "25.37", "0.44", "24.93", "50.28");
    check_period(&model, 4, "25.37", "0.29", "25.08", "25.20");
    check_period(&model, 5, "25.35", "0.15", "25.20", "0.00");
}

/// `test_leap_year_only_actual_for_loan_S1`: 10000 @ 9.482%, ACTUAL/ACTUAL with `FEB_29_PERIOD_ONLY`, six monthly
/// periods Dec-12-2023 → Jun-12-2024 (HALF_EVEN). No period both crosses a year *and* contains Feb 29, so every
/// period uses the simple `rate × actualDays / daysInYear` form, with daysInYear = 366 only for the period that
/// contains Feb 29 (Feb-12 → Mar-12). Exact per-period golden.
#[test]
fn feb29_period_only_s1() {
    let periods = [
        (d(2023, Month::December, 12), d(2024, Month::January, 12)),
        (d(2024, Month::January, 12), d(2024, Month::February, 12)),
        (d(2024, Month::February, 12), d(2024, Month::March, 12)),
        (d(2024, Month::March, 12), d(2024, Month::April, 12)),
        (d(2024, Month::April, 12), d(2024, Month::May, 12)),
        (d(2024, Month::May, 12), d(2024, Month::June, 12)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("10000"),
        &bd("9.482"),
        DayCountKind::ActualFeb29PeriodOnly,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "1713.21", "80.53", "1632.68", "8367.32");
    check_period(&model, 1, "1713.21", "67.38", "1645.83", "6721.49");
    check_period(&model, 2, "1713.21", "50.50", "1662.71", "5058.78");
    check_period(&model, 3, "1713.21", "40.74", "1672.47", "3386.31");
    check_period(&model, 4, "1713.21", "26.39", "1686.82", "1699.49");
    check_period(&model, 5, "1713.18", "13.69", "1699.49", "0.00");
}

/// `test_leap_year_only_actual_for_loan_S2`: 15000 @ 12%, `FEB_29_PERIOD_ONLY`, four monthly periods
/// Jul-23-2024 → Nov-23-2024 — a leap year but **no period contains February**, so every period uses 365 days
/// (the 366 override applies only to a period containing Feb 29). Exact per-period golden.
#[test]
fn feb29_period_only_s2() {
    let periods = [
        (d(2024, Month::July, 23), d(2024, Month::August, 23)),
        (d(2024, Month::August, 23), d(2024, Month::September, 23)),
        (d(2024, Month::September, 23), d(2024, Month::October, 23)),
        (d(2024, Month::October, 23), d(2024, Month::November, 23)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("15000"),
        &bd("12.0"),
        DayCountKind::ActualFeb29PeriodOnly,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "3845.41", "152.88", "3692.53", "11307.47");
    check_period(&model, 1, "3845.41", "115.24", "3730.17", "7577.30");
    check_period(&model, 2, "3845.41", "74.74", "3770.67", "3806.63");
    check_period(&model, 3, "3845.43", "38.80", "3806.63", "0.00");
}

/// `test_leap_year_only_actual_for_loan_S3`: 245000 @ 45%, `FEB_29_PERIOD_ONLY`, six month-end periods
/// Oct-31-2023 → Apr-30-2024. Period 2 (Dec-31 → Jan-31) crosses the year but contains no Feb 29 (→ 365, simple
/// form); period 3 (Jan-31 → Feb-29) contains Feb 29 within one year (→ 366). Exact per-period golden.
#[test]
fn feb29_period_only_s3() {
    let periods = [
        (d(2023, Month::October, 31), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 31)),
        (d(2023, Month::December, 31), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("245000"),
        &bd("45.00"),
        DayCountKind::ActualFeb29PeriodOnly,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "46348.39", "9061.64", "37286.75", "207713.25");
    check_period(&model, 1, "46348.39", "7938.63", "38409.76", "169303.49");
    check_period(&model, 2, "46348.39", "6470.64", "39877.75", "129425.74");
    check_period(&model, 3, "46348.39", "4614.77", "41733.62", "87692.12");
    check_period(&model, 4, "46348.39", "3351.52", "42996.87", "44695.25");
    check_period(&model, 5, "46348.36", "1653.11", "44695.25", "0.00");
}

/// `test_leap_year_only_actual_for_loan_S4`: 2450 @ 9.99%, `FEB_29_PERIOD_ONLY`, six month-end periods
/// Oct-31-2024 → Apr-30-2025 — crosses a leap year into a non-leap year, no period contains Feb 29 (Feb-2025 is
/// not leap), so every period uses 365 days. Exact per-period golden.
#[test]
fn feb29_period_only_s4() {
    let periods = [
        (d(2024, Month::October, 31), d(2024, Month::November, 30)),
        (d(2024, Month::November, 30), d(2024, Month::December, 31)),
        (d(2024, Month::December, 31), d(2025, Month::January, 31)),
        (d(2025, Month::January, 31), d(2025, Month::February, 28)),
        (d(2025, Month::February, 28), d(2025, Month::March, 31)),
        (d(2025, Month::March, 31), d(2025, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("2450"),
        &bd("9.99"),
        DayCountKind::ActualFeb29PeriodOnly,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "420.24", "20.12", "400.12", "2049.88");
    check_period(&model, 1, "420.24", "17.39", "402.85", "1647.03");
    check_period(&model, 2, "420.24", "13.97", "406.27", "1240.76");
    check_period(&model, 3, "420.24", "9.51", "410.73", "830.03");
    check_period(&model, 4, "420.24", "7.04", "413.20", "416.83");
    check_period(&model, 5, "420.25", "3.42", "416.83", "0.00");
}

/// `test_leap_year_only_actual_for_loan_S5`: 5000 @ 7%, `FEB_29_PERIOD_ONLY`, **repay every 2 months**, six
/// bi-monthly periods Oct-29-2022 → Oct-29-2023 — no leap year at all (365 throughout). Confirms the actual/actual
/// day-count ignores `repay_every`. Exact per-period golden.
#[test]
fn feb29_period_only_s5() {
    let periods = [
        (d(2022, Month::October, 29), d(2022, Month::December, 29)),
        (d(2022, Month::December, 29), d(2023, Month::February, 28)),
        (d(2023, Month::February, 28), d(2023, Month::April, 29)),
        (d(2023, Month::April, 29), d(2023, Month::June, 29)),
        (d(2023, Month::June, 29), d(2023, Month::August, 29)),
        (d(2023, Month::August, 29), d(2023, Month::October, 29)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("5000"),
        &bd("7.00"),
        DayCountKind::ActualFeb29PeriodOnly,
        2,
        RoundingMode::HalfEven,
    );
    check_period(&model, 0, "867.68", "58.49", "809.19", "4190.81");
    check_period(&model, 1, "867.68", "49.03", "818.65", "3372.16");
    check_period(&model, 2, "867.68", "38.80", "828.88", "2543.28");
    check_period(&model, 3, "867.68", "29.75", "837.93", "1705.35");
    check_period(&model, 4, "867.68", "19.95", "847.73", "857.62");
    check_period(&model, 5, "867.65", "10.03", "857.62", "0.00");
}

/// `test_disbursedAmt100_dayInYearsActual_daysInMonthActual_repayEvery1Month`: 100 @ 9.4822%, ACTUAL/ACTUAL, six
/// monthly periods on the 12th from Dec-12-2023 (crossing 2023→2024, varying month lengths). EMI 17.13. Exercises the
/// year-boundary partial split (rp0 Dec12-Jan12) at repayment- and interest-period granularity.
#[test]
fn repayment_schedule_actual_actual_monthly_12th() {
    let periods = [
        (d(2023, Month::December, 12), d(2024, Month::January, 12)),
        (d(2024, Month::January, 12), d(2024, Month::February, 12)),
        (d(2024, Month::February, 12), d(2024, Month::March, 12)),
        (d(2024, Month::March, 12), d(2024, Month::April, 12)),
        (d(2024, Month::April, 12), d(2024, Month::May, 12)),
        (d(2024, Month::May, 12), d(2024, Month::June, 12)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("100"),
        &bd("9.4822"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    assert_eq!(
        fc_money::decimal::to_plain_string(&model.periods[0].interest_periods[1].rate_factor),
        "0.008044857759"
    );
    check_period(&model, 0, "17.13", "0.80", "16.33", "83.67");
    check_period(&model, 1, "17.13", "0.67", "16.46", "67.21");
    check_period(&model, 2, "17.13", "0.50", "16.63", "50.58");
    check_period(&model, 3, "17.13", "0.41", "16.72", "33.86");
    check_period(&model, 4, "17.13", "0.26", "16.87", "16.99");
    check_period(&model, 5, "17.13", "0.14", "16.99", "0.00");
}

/// `test_actual_actual_repayment_schedule_disbursement_near_month_end_repay_every_2_months`: 5000 @ 7%, ACTUAL/ACTUAL,
/// six bi-monthly periods Oct-29-2022 → Oct-29-2023 (each crosses a year boundary), HALF_UP. EMI 867.68. Exercises the
/// ACTUAL/ACTUAL partial split on bi-monthly periods (equals the simple form here since both years are 365 days).
#[test]
fn repayment_schedule_actual_actual_bimonthly() {
    let periods = [
        (d(2022, Month::October, 29), d(2022, Month::December, 29)),
        (d(2022, Month::December, 29), d(2023, Month::February, 28)),
        (d(2023, Month::February, 28), d(2023, Month::April, 29)),
        (d(2023, Month::April, 29), d(2023, Month::June, 29)),
        (d(2023, Month::June, 29), d(2023, Month::August, 29)),
        (d(2023, Month::August, 29), d(2023, Month::October, 29)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("5000"),
        &bd("7"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "867.68", "58.49", "809.19", "4190.81");
    check_period(&model, 1, "867.68", "49.03", "818.65", "3372.16");
    check_period(&model, 2, "867.68", "38.80", "828.88", "2543.28");
    check_period(&model, 3, "867.68", "29.75", "837.93", "1705.35");
    check_period(&model, 4, "867.68", "19.95", "847.73", "857.62");
    check_period(&model, 5, "867.65", "10.03", "857.62", "0.00");
}

/// Six monthly periods from 2024-01-01.
fn monthly_periods_2024_six() -> Vec<(Date, Date)> {
    (0..6)
        .map(|i| {
            (
                first_of(2024, month_of(1 + i)),
                first_of(2024, month_of(2 + i)),
            )
        })
        .collect()
}
fn month_of(n: u8) -> Month {
    Month::try_from(n).unwrap()
}

/// 12 monthly periods from 2024-01-01.
fn monthly_periods_2024() -> Vec<(Date, Date)> {
    let months = [
        (2024, Month::January),
        (2024, Month::February),
        (2024, Month::March),
        (2024, Month::April),
        (2024, Month::May),
        (2024, Month::June),
        (2024, Month::July),
        (2024, Month::August),
        (2024, Month::September),
        (2024, Month::October),
        (2024, Month::November),
        (2024, Month::December),
        (2025, Month::January),
    ];
    (0..12)
        .map(|i| {
            (
                first_of(months[i].0, months[i].1),
                first_of(months[i + 1].0, months[i + 1].1),
            )
        })
        .collect()
}

fn day_count() -> DayCountKind {
    DayCountKind::Days360Days30 { repay_every: 1 }
}

/// `test_actual_actual_repayment_schedule_across_multiple_years` (generation block, before the payment): 5000 @ 9.99%
/// nominal, ACTUAL/ACTUAL, six monthly periods crossing the 2023→2024 boundary, HALF_UP. Exact per-period golden,
/// exercising the year-boundary partial-period split.
#[test]
fn repayment_schedule_actual_actual_across_years() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "857.71", "41.05", "816.66", "4183.34");
    check_period(&model, 1, "857.71", "35.45", "822.26", "3361.08");
    check_period(&model, 2, "857.71", "28.44", "829.27", "2531.81");
    check_period(&model, 3, "857.71", "20.04", "837.67", "1694.14");
    check_period(&model, 4, "857.71", "14.33", "843.38", "850.76");
    check_period(&model, 5, "857.73", "6.97", "850.76", "0.00");
}

/// `test_actual_actual_fraction_period_calculation_with_interestRecognitionFromDisbursementDate_true`: same loan as the
/// across-years case but with `isInterestRecognitionOnDisbursementDate = true`, so the year-boundary split is at Jan 1
/// (not Dec 31) — yielding EMI 857.72 (vs 857.71). Exact per-period golden.
#[test]
fn repayment_schedule_actual_actual_recognition_on_disbursement() {
    let periods = [
        (d(2023, Month::November, 13), d(2023, Month::December, 13)),
        (d(2023, Month::December, 13), d(2024, Month::January, 13)),
        (d(2024, Month::January, 13), d(2024, Month::February, 13)),
        (d(2024, Month::February, 13), d(2024, Month::March, 13)),
        (d(2024, Month::March, 13), d(2024, Month::April, 13)),
        (d(2024, Month::April, 13), d(2024, Month::May, 13)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("5000"),
        &bd("9.99"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: true,
        },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "857.72", "41.05", "816.67", "4183.33");
    check_period(&model, 1, "857.72", "35.46", "822.26", "3361.07");
    check_period(&model, 2, "857.72", "28.44", "829.28", "2531.79");
    check_period(&model, 3, "857.72", "20.04", "837.68", "1694.11");
    check_period(&model, 4, "857.72", "14.33", "843.39", "850.72");
    check_period(&model, 5, "857.69", "6.97", "850.72", "0.00");
}

/// `test_actual_actual_repayment_schedule_disbursement_near_month_end`: 1500 @ 9.4822%, ACTUAL/ACTUAL, six
/// near-month-end periods crossing 2021→2022, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_actual_actual_near_month_end() {
    let periods = [
        (d(2021, Month::October, 30), d(2021, Month::November, 30)),
        (d(2021, Month::November, 30), d(2021, Month::December, 30)),
        (d(2021, Month::December, 30), d(2022, Month::January, 30)),
        (d(2022, Month::January, 30), d(2022, Month::February, 28)),
        (d(2022, Month::February, 28), d(2022, Month::March, 30)),
        (d(2022, Month::March, 30), d(2022, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("1500"),
        &bd("9.4822"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "256.95", "12.08", "244.87", "1255.13");
    check_period(&model, 1, "256.95", "9.78", "247.17", "1007.96");
    check_period(&model, 2, "256.95", "8.12", "248.83", "759.13");
    check_period(&model, 3, "256.95", "5.72", "251.23", "507.90");
    check_period(&model, 4, "256.95", "3.96", "252.99", "254.91");
    check_period(&model, 5, "256.96", "2.05", "254.91", "0.00");
}

/// `test_actual_actual_repayment_schedule_disbursement_month_end` (generation block): 245000 @ 45% nominal,
/// ACTUAL/ACTUAL, six month-end periods crossing 2023→2024, HALF_UP. Exact per-period golden.
#[test]
fn repayment_schedule_actual_actual_month_end() {
    let periods = [
        (d(2023, Month::October, 31), d(2023, Month::November, 30)),
        (d(2023, Month::November, 30), d(2023, Month::December, 31)),
        (d(2023, Month::December, 31), d(2024, Month::January, 31)),
        (d(2024, Month::January, 31), d(2024, Month::February, 29)),
        (d(2024, Month::February, 29), d(2024, Month::March, 31)),
        (d(2024, Month::March, 31), d(2024, Month::April, 30)),
    ];
    let model = generate_declining_balance(
        &periods,
        &bd("245000"),
        &bd("45.0"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfUp,
    );
    check_period(&model, 0, "46343.27", "9061.64", "37281.63", "207718.37");
    check_period(&model, 1, "46343.27", "7938.83", "38404.44", "169313.93");
    check_period(&model, 2, "46343.27", "6453.36", "39889.91", "129424.02");
    check_period(&model, 3, "46343.27", "4614.71", "41728.56", "87695.46");
    check_period(&model, 4, "46343.27", "3342.49", "43000.78", "44694.68");
    check_period(&model, 5, "46343.25", "1648.57", "44694.68", "0.00");
}

#[test]
fn due_amounts_first_period() {
    let model = generate_declining_balance(
        &monthly_periods_2024(),
        &bd("100"),
        &bd("7.0"),
        day_count(),
        2,
        RoundingMode::HalfEven,
    );
    let (from, due) = monthly_periods_2024()[0];
    let d = model.due_amounts(from, due).expect("first period");
    // EMI 8.65; interest on 100 @ 0.005833333333 → 0.58; principal = 8.65 − 0.58 = 8.07.
    assert_eq!(d.emi.to_plain_string(), "8.65");
    assert_eq!(d.due_interest.to_plain_string(), "0.58");
    assert_eq!(d.due_principal.to_plain_string(), "8.07");
}

#[test]
fn emi_calculator_performance_fresh_disbursement() {
    let day_count = day_count();
    let model = generate_declining_balance(
        &monthly_periods_2024(),
        &bd("100"),
        &bd("7.0"),
        day_count,
        2,
        RoundingMode::HalfEven,
    );

    assert_eq!(model.periods.len(), 12, "12 repayment periods");
    assert_eq!(
        model.loan_term_in_days(),
        366,
        "loan term in days (2024 leap year)"
    );

    let n = model.periods.len();
    for (i, p) in model.periods.iter().enumerate() {
        assert!(
            p.due_principal.is_greater_than_zero(),
            "period {} due principal > 0",
            i + 1
        );
        assert!(
            p.due_interest.is_greater_than_zero(),
            "period {} due interest > 0",
            i + 1
        );
        if i == n - 1 {
            assert_eq!(
                p.outstanding_balance.to_plain_string(),
                "0.00",
                "final period outstanding == 0"
            );
        } else {
            assert_eq!(p.emi.to_plain_string(), "8.65", "period {} EMI", i + 1);
            assert!(
                p.outstanding_balance.is_greater_than_zero(),
                "period {} outstanding > 0",
                i + 1
            );
        }
    }
}

/// `test_sameAsRepayment_days_repay_every_6_periods_5` (FLAT, SAME_AS_REPAYMENT_PERIOD): 1000 @ 12.2%, ACTUAL/ACTUAL,
/// five 6-day periods from 2024-01-01 — flat interest 1000·0.122·6/366 = 2.00 per period, EMI 202.00, principal 200.00.
#[test]
fn same_as_repayment_days_repay_every_6_periods_5() {
    let periods: Vec<(Date, Date)> = (0..5)
        .map(|i| {
            (
                d(2024, Month::January, 1 + (i * 6) as u8),
                d(2024, Month::January, 1 + ((i + 1) * 6) as u8),
            )
        })
        .collect();
    let model = generate_flat(
        &periods,
        &bd("1000"),
        &bd("12.2"),
        DayCountKind::ActualActual {
            recognition_on_disbursement: false,
        },
        2,
        RoundingMode::HalfEven,
    );
    for i in 0..5 {
        let out = format!("{}.00", 800 - i * 200);
        check_period(&model, i as usize, "202.00", "2.00", "200.00", &out);
    }
}

/// `test_sameAsRepayment_week_repay_every_1_periods_10` (FLAT, SAME_AS_REPAYMENT_PERIOD): 1000 @ 5.2%, DAYS_364, ten
/// weekly periods from 2024-01-01 — flat interest 1000·0.052·7/364 = 1.00 per period, EMI 101.00, principal 100.00.
#[test]
fn same_as_repayment_week_repay_every_1_periods_10() {
    let periods: Vec<(Date, Date)> = (0..10)
        .map(|i| {
            (
                d(2024, Month::January, 1) + time::Duration::weeks(i),
                d(2024, Month::January, 1) + time::Duration::weeks(i + 1),
            )
        })
        .collect();
    let model = generate_flat(
        &periods,
        &bd("1000"),
        &bd("5.2"),
        DayCountKind::ActualFixedYear { days_in_year: 364 },
        2,
        RoundingMode::HalfEven,
    );
    for i in 0..10 {
        let out = format!("{}.00", 900 - i * 100);
        check_period(&model, i as usize, "101.00", "1.00", "100.00", &out);
    }
}

/// `test_sameAsRepayment_month_repay_every_2_periods_8` (FLAT, SAME_AS_REPAYMENT_PERIOD): 1000 @ 12%, eight 2-month
/// periods from 2024-01-01 — sameAsRepayment monthly rate factor = rate·repayEvery/12 = 0.02 → flat interest 20.00 per
/// period, EMI 145.00, principal 125.00.
#[test]
fn same_as_repayment_month_repay_every_2_periods_8() {
    let periods: Vec<(Date, Date)> = (0..8)
        .map(|i| (add_months_2(i), add_months_2(i + 1)))
        .collect();
    let model = generate_flat(
        &periods,
        &bd("1000"),
        &bd("12.0"),
        DayCountKind::SameAsRepaymentMonths { repay_every: 2 },
        2,
        RoundingMode::HalfEven,
    );
    for i in 0..8 {
        let out = format!("{}.00", 875 - i * 125);
        check_period(&model, i as usize, "145.00", "20.00", "125.00", &out);
    }
}

/// 2024-01-01 plus `2*k` months, for the 2-month sameAsRepayment schedule above.
fn add_months_2(k: i64) -> Date {
    add_months_1(2 * k)
}

/// `test_sameAsRepayment_month_repay_every_1_periods_20` (FLAT, SAME_AS_REPAYMENT_PERIOD): 1000 @ 12%, twenty monthly
/// periods from 2024-01-01 — flat interest 10.00 per period (rate/12), EMI 60.00, equal principal 50.00.
#[test]
fn same_as_repayment_month_repay_every_1_periods_20() {
    let periods: Vec<(Date, Date)> = (0..20)
        .map(|i| (add_months_1(i), add_months_1(i + 1)))
        .collect();
    let model = generate_flat(
        &periods,
        &bd("1000"),
        &bd("12.0"),
        DayCountKind::SameAsRepaymentMonths { repay_every: 1 },
        2,
        RoundingMode::HalfEven,
    );
    for i in 0..20 {
        let out = format!("{}.00", 950 - i * 50);
        check_period(&model, i as usize, "60.00", "10.00", "50.00", &out);
    }
}

/// 2024-01-01 plus `k` months, for the 20-period monthly sameAsRepayment schedule above.
fn add_months_1(k: i64) -> Date {
    let base_month0 = k;
    let year = 2024 + (base_month0 / 12) as i32;
    let month0 = (base_month0 % 12) as u8;
    let month = Month::try_from(month0 + 1).unwrap();
    Date::from_calendar_date(year, month, 1).unwrap()
}
