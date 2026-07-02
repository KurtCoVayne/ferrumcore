//! Integration: the real `fc-loan-emi` engine wired behind the Advanced processor's [`EmiCalculator`] seam via
//! [`ProgressiveEmiCalculator`]. Confirms the seam operations (driven as `&mut dyn EmiCalculator`, exactly as the
//! Advanced processor calls them) actually mutate the underlying progressive interest schedule.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::ProgressiveEmiCalculator;
use fc_loan_allocation::advanced::{
    AdvancedInstallment, AllocationType, EmiCalculator, ScheduleModel,
};
use fc_loan_allocation::emi_adapter::{
    RestFrequencyType, calculate_is_prepay_attempt,
    calculate_new_pay_date_in_case_of_in_advance_payment,
    should_recalculate_till_installment_due_date, update_installments_by_repayment_periods,
    update_repayment_period_balances,
};
use fc_loan_allocation::installment::Installment;
use fc_loan_emi::recalc::new_declining_balance;
use fc_money::Money;
use std::str::FromStr;
use time::{Date, Month};

fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}
fn d(y: i32, m: Month, day: u8) -> Date {
    Date::from_calendar_date(y, m, day).unwrap()
}
fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &bd(s))
}

/// A 100 @ 7% DAYS_360/30 declining loan (6 monthly periods, EMI 17.01) driven through the seam: paying period 0's
/// principal + interest via `&mut dyn EmiCalculator` marks it fully paid in the real engine, and the outstanding
/// cascades to the next period — proving the production calculator is wired correctly.
#[test]
fn real_engine_behind_emi_seam_records_payment() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ];
    let model = new_declining_balance(
        &periods,
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    // Fresh schedule: period 0 due is 16.43 principal + 0.58 interest, EMI 17.01, nothing paid yet.
    assert_eq!(model.period_due_principal(0).to_plain_string(), "16.43");
    assert_eq!(model.period_due_interest(0).to_plain_string(), "0.58");
    assert!(!model.period_fully_paid(0));

    let mut calc = ProgressiveEmiCalculator::new(model);
    let schedule = ScheduleModel {
        maturity_date: d(2024, Month::July, 1),
    };
    let jan1 = d(2024, Month::January, 1);
    let feb1 = d(2024, Month::February, 1);

    // Drive the payment through the seam exactly as the Advanced processor does (as a trait object).
    let seam: &mut dyn EmiCalculator = &mut calc;
    seam.pay_principal(&schedule, jan1, feb1, feb1, &m("16.43"));
    seam.pay_interest(&schedule, jan1, feb1, feb1, &m("0.58"));

    // The real engine recorded the payment on period 0.
    assert!(
        calc.model().period_fully_paid(0),
        "period 0 fully paid after seam payment"
    );
    // ... and the remaining schedule is intact (period 1 still carries its EMI-derived due principal).
    assert!(
        !calc.model().period_fully_paid(1),
        "period 1 still outstanding"
    );
    assert_eq!(
        calc.model().period_due_interest(1).to_plain_string(),
        "0.49"
    );
}

/// A second disbursement driven through the seam re-amortizes the real engine (the merge path), proving
/// `add_disbursement` is wired, not a no-op.
#[test]
fn real_engine_behind_emi_seam_records_disbursement() {
    let periods = [
        (d(2024, Month::January, 1), d(2024, Month::February, 1)),
        (d(2024, Month::February, 1), d(2024, Month::March, 1)),
        (d(2024, Month::March, 1), d(2024, Month::April, 1)),
        (d(2024, Month::April, 1), d(2024, Month::May, 1)),
        (d(2024, Month::May, 1), d(2024, Month::June, 1)),
        (d(2024, Month::June, 1), d(2024, Month::July, 1)),
    ];
    let model = new_declining_balance(
        &periods,
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let emi_before = model.period_emi(0);

    let mut calc = ProgressiveEmiCalculator::new(model);
    let schedule = ScheduleModel {
        maturity_date: d(2024, Month::July, 1),
    };
    let seam: &mut dyn EmiCalculator = &mut calc;
    // A 2nd disbursement on a period due date re-amortizes the schedule upward.
    seam.add_disbursement(&schedule, d(2024, Month::March, 1), &m("100.0"));

    assert!(
        calc.model().period_emi(2).is_greater_than(&emi_before),
        "2nd disbursement raised the EMI in the real engine"
    );
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

/// An installment with `principal`/`interest` charged, `from`/`due` bounding the period.
fn installment(from: Date, due: Date, principal: &str, interest: &str) -> AdvancedInstallment {
    AdvancedInstallment::new(
        Installment::new(
            2,
            RoundingMode::HalfEven,
            due,
            bd(principal),
            bd(interest),
            BigDecimal::from(0),
            BigDecimal::from(0),
        ),
        false,
    )
    .with_from_date(from)
}

/// `updateInstallmentsByRepaymentPeriods` (src 2918) against the golden-verified 100 @ 7% engine schedule: every
/// installment's charged principal / interest is overwritten with the engine's bit-exact per-period values
/// (16.43/0.58 … 16.90/0.10), a down-payment row with the same dates is skipped (the sync matches the row after it),
/// and fresh credited amounts sync to zero.
#[test]
fn model_installment_sync_writes_engine_values() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );

    // A down-payment row sharing period 0's dates (must be skipped) + the six schedule rows, all zero-charged.
    let mut down_payment = installment(
        d(2024, Month::January, 1),
        d(2024, Month::February, 1),
        "0",
        "0",
    );
    down_payment.is_down_payment = true;
    let mut installments = vec![down_payment];
    for &(from, due) in &six_monthly() {
        installments.push(installment(from, due, "0", "0"));
    }

    update_installments_by_repayment_periods(&model, &mut installments);

    // The down-payment row stays untouched.
    assert_eq!(
        installments[0].installment().principal().to_plain_string(),
        "0.00"
    );
    // The schedule rows carry the engine's golden values.
    let expected = [
        ("16.43", "0.58"),
        ("16.52", "0.49"),
        ("16.62", "0.39"),
        ("16.72", "0.29"),
        ("16.81", "0.20"),
        ("16.90", "0.10"),
    ];
    for (i, (principal, interest)) in expected.iter().enumerate() {
        let inst = installments[i + 1].installment();
        assert_eq!(
            inst.principal().to_plain_string(),
            *principal,
            "row {i} principal"
        );
        assert_eq!(
            inst.interest_charged().to_plain_string(),
            *interest,
            "row {i} interest"
        );
        assert_eq!(
            installments[i + 1].credited_principal().to_plain_string(),
            "0.00",
            "row {i} credited principal"
        );
    }
}

/// `updateRepaymentPeriodBalances` (src 2933) against the golden `getDueAmounts` values: an IN_ADVANCE_PRINCIPAL
/// refresh on the period due date writes the full due principal (16.52), an IN_ADVANCE_INTEREST refresh mid-period
/// writes the prorated interest (0.24 on Feb 15) — each leaving the other component untouched; neither case skips.
#[test]
fn update_repayment_period_balances_refreshes_from_due_amounts() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let feb1 = d(2024, Month::February, 1);
    let mar1 = d(2024, Month::March, 1);
    let feb15 = d(2024, Month::February, 15);

    let mut inst = installment(feb1, mar1, "100", "10");
    let skip = update_repayment_period_balances(&model, &mut inst, AllocationType::Principal, mar1);
    assert!(!skip);
    assert_eq!(inst.installment().principal().to_plain_string(), "16.52");
    assert_eq!(
        inst.installment().interest_charged().to_plain_string(),
        "10.00"
    );

    let mut inst = installment(feb1, mar1, "100", "10");
    let skip = update_repayment_period_balances(&model, &mut inst, AllocationType::Interest, feb15);
    assert!(!skip);
    assert_eq!(
        inst.installment().interest_charged().to_plain_string(),
        "0.24"
    );
    assert_eq!(inst.installment().principal().to_plain_string(), "100.00");
}

/// The in-advance pay-date + prepay helpers: `shouldRecalculateTillInstallmentDueDate` (src 2977) — DAILY requires a
/// prepay attempt AND the till-rest-frequency strategy, SAME_AS_REPAYMENT_PERIOD follows the strategy alone;
/// `calculateNewPayDateInCaseOfInAdvancePayment` (src 2967) — snaps to the due date only when recalculating and the
/// txn is inside `(from, due]`; `calculateIsPrepayAttempt` (src 2949) — amount covers the outstanding till the
/// installment due date (100 + 2.05 golden total at maturity; 100 + 0.58 at the first due date).
#[test]
fn pay_date_and_prepay_helpers() {
    use RestFrequencyType::{Daily, SameAsRepaymentPeriod};
    assert!(should_recalculate_till_installment_due_date(
        Daily, true, true
    ));
    assert!(!should_recalculate_till_installment_due_date(
        Daily, true, false
    ));
    assert!(!should_recalculate_till_installment_due_date(
        Daily, false, true
    ));
    assert!(should_recalculate_till_installment_due_date(
        SameAsRepaymentPeriod,
        true,
        false
    ));
    assert!(!should_recalculate_till_installment_due_date(
        SameAsRepaymentPeriod,
        false,
        true
    ));

    let feb1 = d(2024, Month::February, 1);
    let feb15 = d(2024, Month::February, 15);
    let mar1 = d(2024, Month::March, 1);
    // Recalculating + txn inside (from, due] → due date.
    assert_eq!(
        calculate_new_pay_date_in_case_of_in_advance_payment(feb15, feb1, mar1, true),
        mar1
    );
    // txn ON the from date is outside the period (from-exclusive) → txn date.
    assert_eq!(
        calculate_new_pay_date_in_case_of_in_advance_payment(feb1, feb1, mar1, true),
        feb1
    );
    // Not recalculating → txn date.
    assert_eq!(
        calculate_new_pay_date_in_case_of_in_advance_payment(feb15, feb1, mar1, false),
        feb15
    );

    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let jul1 = d(2024, Month::July, 1);
    // Outstanding till maturity = 100.00 principal + 2.05 interest (the golden schedule totals).
    assert!(calculate_is_prepay_attempt(&model, jul1, &m("102.05")));
    assert!(!calculate_is_prepay_attempt(&model, jul1, &m("102.04")));
    // Outstanding till the first due date = 100.00 + 0.58 (only period 0's golden interest has accrued).
    assert!(calculate_is_prepay_attempt(&model, feb1, &m("100.58")));
    assert!(!calculate_is_prepay_attempt(&model, feb1, &m("100.57")));
}
