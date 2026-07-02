//! Integration: the real `fc-loan-emi` engine wired behind the Advanced processor's [`EmiCalculator`] seam via
//! [`ProgressiveEmiCalculator`]. Confirms the seam operations (driven as `&mut dyn EmiCalculator`, exactly as the
//! Advanced processor calls them) actually mutate the underlying progressive interest schedule.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_allocation::ProgressiveEmiCalculator;
use fc_loan_allocation::advanced::{
    AdvancedInstallment, AllocationType, EmiCalculator, ScheduleModel,
};
use fc_loan_allocation::advanced::{DueType, PaymentAllocation};
use fc_loan_allocation::emi_adapter::{
    ChargeOffBehaviour, RestFrequencyType, calculate_is_prepay_attempt,
    calculate_new_pay_date_in_case_of_in_advance_payment, charge_off_is_in_effect,
    handle_capitalized_income_progressive, handle_contract_termination,
    handle_payment_allocation_progressive, handle_re_age_progressive,
    interest_refund_amount_with_charge_off, interest_refund_amount_without_charge_off,
    interest_till_charge_off, should_recalculate_till_installment_due_date,
    update_installments_by_repayment_periods, update_repayment_period_balances,
};
use fc_loan_allocation::installment::Installment;
use fc_loan_emi::recalc::{DayCount, new_declining_balance, new_declining_balance_with_day_count};
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

/// `handlingPaymentAllocationForInterestBearingProgressiveLoan` (src 2884) against the real engine, replaying the
/// `test_payoff_on0215_*` stage-2 golden as two IN_ADVANCE allocations on Feb 15 (period 1, Feb 1–Mar 1):
/// the balance refresh writes the golden mid-period due amounts (16.77 / 0.24) onto the installment, the engine
/// records `payPrincipal`/`payInterest` at the Feb 15 pay date (the exact golden calls), and the model→installment
/// sync propagates the golden cascaded schedule (period 2 → 16.42 / 0.59, period 3 → 16.72 / 0.29). Also covers the
/// after-maturity skip branch (plain component pay, engine untouched).
#[test]
fn progressive_payment_allocation_replays_payoff_golden() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let feb15 = d(2024, Month::February, 15);

    // Installments mirroring the schedule rows, synced to the fresh engine values.
    let mut installments: Vec<AdvancedInstallment> = six_monthly()
        .iter()
        .map(|&(from, due)| installment(from, due, "0", "0"))
        .collect();
    update_installments_by_repayment_periods(&model, &mut installments);

    // IN_ADVANCE principal on Feb 15 against period 1 — no recalculation config → pay date stays Feb 15.
    let out = handle_payment_allocation_progressive(
        &mut model,
        &mut installments,
        1,
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::InAdvance,
        },
        feb15,
        &m("16.77"),
        false,
    );
    assert!(!out.skip);
    assert_eq!(out.paid.to_plain_string(), "16.77");

    // IN_ADVANCE interest for the prorated 0.24.
    let out = handle_payment_allocation_progressive(
        &mut model,
        &mut installments,
        1,
        PaymentAllocation {
            allocation_type: AllocationType::Interest,
            due_type: DueType::InAdvance,
        },
        feb15,
        &m("0.24"),
        false,
    );
    assert!(!out.skip);
    assert_eq!(out.paid.to_plain_string(), "0.24");

    // The engine reproduced the payoff stage-2 golden: period 1 fully paid at 16.77/0.24, the cascade re-priced
    // period 2 to 16.42/0.59 and period 3 to 16.72/0.29 — and the sync wrote those values onto the installments.
    assert!(model.period_fully_paid(1));
    let expect = [
        (1usize, "16.77", "0.24"),
        (2, "16.42", "0.59"),
        (3, "16.72", "0.29"),
    ];
    for (i, principal, interest) in expect {
        assert_eq!(
            model.period_due_principal(i).to_plain_string(),
            principal,
            "engine period {i} principal"
        );
        let inst = installments[i].installment();
        assert_eq!(
            inst.principal().to_plain_string(),
            principal,
            "row {i} principal"
        );
        assert_eq!(
            inst.interest_charged().to_plain_string(),
            interest,
            "row {i} interest"
        );
    }
    assert_eq!(
        installments[1]
            .installment()
            .principal_outstanding()
            .to_plain_string(),
        "0.00"
    );

    run_after_maturity_skip_branch(&mut model);
}

/// `handleReAgeEqualAmortizationEMICalculator` (src 3806) core, replaying the FULL_INTEREST re-age golden
/// (`reage_equal_amortization_preserves_outstanding`: 100 @ 15.678% DAYS_365, re-age Apr 15 → 6 monthly installments
/// from Apr 20; precalculated outstanding = 100.00 / 4.62): the transaction components carry the golden precalculated
/// amounts, past rows close at their paid amounts, the re-aged rows carry the engine's schedule, and the
/// outstanding-preservation invariant (Σ outstanding = 100.00 / 4.62) survives the rebuild.
#[test]
fn progressive_re_age_replays_engine_golden() {
    let mut model = new_declining_balance_with_day_count(
        &six_monthly(),
        &bd("100"),
        &bd("15.678"),
        DayCount::ActualFixedYear { days_in_year: 365 },
        2,
        RoundingMode::HalfEven,
    );
    let mut installments: Vec<AdvancedInstallment> = six_monthly()
        .iter()
        .map(|&(from, due)| installment(from, due, "0", "0"))
        .collect();
    update_installments_by_repayment_periods(&model, &mut installments);

    let apr15 = d(2024, Month::April, 15);
    let out = handle_re_age_progressive(
        &mut model,
        &mut installments,
        apr15,
        d(2024, Month::April, 20),
        6,
        1,
        true, // FULL_INTEREST
    );

    // The golden precalculated re-age amounts stamp the transaction.
    assert_eq!(out.principal.to_plain_string(), "100.00");
    assert_eq!(out.interest.to_plain_string(), "4.62");
    assert!(!out.reversed);

    // Structure: every re-aged model period materialized a row; past rows closed at their (zero) paid amounts.
    let re_aged_periods = model.periods.iter().filter(|p| p.due >= apr15).count();
    let re_aged_rows = installments.iter().filter(|i| i.is_re_aged).count();
    assert_eq!(re_aged_rows, re_aged_periods, "one row per re-aged period");
    for inst in installments.iter().filter(|i| !i.is_re_aged) {
        assert!(inst.due_date() < apr15, "only past rows survive un-re-aged");
        assert_eq!(
            inst.installment().principal().to_plain_string(),
            "0.00",
            "past row closes at its paid principal"
        );
        assert_eq!(
            inst.installment().interest_charged().to_plain_string(),
            "0.00",
            "past row closes at its paid interest"
        );
    }

    // The outstanding-preservation invariant threads through the rebuilt schedule.
    let zero = m("0");
    let (outstanding_principal, outstanding_interest) =
        installments
            .iter()
            .fold((zero.clone(), zero), |(p, i), inst| {
                (
                    p.plus(&inst.installment().principal_outstanding()),
                    i.plus(&inst.installment().interest_outstanding()),
                )
            });
    assert_eq!(outstanding_principal.to_plain_string(), "100.00");
    assert_eq!(outstanding_interest.to_plain_string(), "4.62");

    // reprocessInstallments left the rows ordered and stitched (each from == previous due).
    for w in installments.windows(2) {
        assert!(w[0].due_date() <= w[1].due_date(), "ordered by due date");
        assert_eq!(
            w[1].from_date(),
            w[0].due_date(),
            "from stitched to previous due"
        );
    }
}

fn run_after_maturity_skip_branch(model: &mut fc_loan_emi::recalc::Model) {
    // After-maturity skip branch: an installment due past the model maturity pays the component only.
    let emi_before = model.period_emi(4);
    let mut extra = vec![installment(
        d(2024, Month::July, 1),
        d(2024, Month::August, 1),
        "50",
        "0",
    )];
    let out = handle_payment_allocation_progressive(
        model,
        &mut extra,
        0,
        PaymentAllocation {
            allocation_type: AllocationType::Principal,
            due_type: DueType::Due,
        },
        d(2024, Month::August, 1),
        &m("20"),
        false,
    );
    assert!(out.skip);
    assert_eq!(out.paid.to_plain_string(), "20.00");
    assert_eq!(
        extra[0]
            .installment()
            .principal_completed()
            .to_plain_string(),
        "20.00"
    );
    assert_eq!(
        model.period_emi(4).to_plain_string(),
        emi_before.to_plain_string(),
        "engine untouched by the skip branch"
    );
}

/// `chargeOffIsInEffect` (src 684) + `getInterestTillChargeOffForPeriod` (src 3631) + `handleInterestRefund`'s
/// charge-off branch (src 641-655) — verified against the captured Java goldens (`fcCaptureInterestRefundChargeOff`,
/// instrumented FCITCO): a past installment (100/10, due Mar 1) and an in-period one (100/12, Mar 1–Apr 1) with the
/// charge-off on Mar 16 give 10.00 + 12.00 = 22.00 for ZERO_INTEREST (full outstanding) and 10.00 + 5.81 = 15.81 for
/// REGULAR (12/31 days × 15 at MathContext(12, HALF_EVEN)); the refund amount is 22.00 for ZERO_INTEREST and 22.00
/// (Σ interest charged) for REGULAR-on-progressive.
#[test]
fn interest_till_charge_off_matches_capture() {
    let mar16 = d(2023, Month::March, 16);
    let rows = [
        installment(
            d(2023, Month::February, 1),
            d(2023, Month::March, 1),
            "100",
            "10",
        ),
        installment(
            d(2023, Month::March, 1),
            d(2023, Month::April, 1),
            "100",
            "12",
        ),
    ];

    // charge_off_is_in_effect: ctx flag, earlier date, equal-date tie (stable sort → charge-off first), later date.
    assert!(charge_off_is_in_effect(true, None, mar16));
    assert!(charge_off_is_in_effect(
        false,
        Some(d(2023, Month::March, 1)),
        mar16
    ));
    assert!(charge_off_is_in_effect(false, Some(mar16), mar16));
    assert!(!charge_off_is_in_effect(
        false,
        Some(d(2023, Month::March, 20)),
        mar16
    ));
    assert!(!charge_off_is_in_effect(false, None, mar16));

    // Non-recalculation branches (FCITCO goldens).
    let zero_interest =
        interest_till_charge_off(None, &rows, mar16, ChargeOffBehaviour::ZeroInterest);
    assert_eq!(zero_interest.to_plain_string(), "22.00");
    let accelerate =
        interest_till_charge_off(None, &rows, mar16, ChargeOffBehaviour::AccelerateMaturity);
    assert_eq!(
        accelerate.to_plain_string(),
        "22.00",
        "same branch as ZERO_INTEREST"
    );
    let regular = interest_till_charge_off(None, &rows, mar16, ChargeOffBehaviour::Regular);
    assert_eq!(
        regular.to_plain_string(),
        "15.81",
        "10.00 past + 5.81 prorated"
    );

    // Refund amount (FCIR goldens): ZERO_INTEREST → till-charge-off; REGULAR on progressive → Σ interest charged.
    let refund_zero = interest_refund_amount_with_charge_off(
        None,
        &rows,
        mar16,
        ChargeOffBehaviour::ZeroInterest,
        true,
    );
    assert_eq!(refund_zero.to_plain_string(), "22.00");
    let refund_regular = interest_refund_amount_with_charge_off(
        None,
        &rows,
        mar16,
        ChargeOffBehaviour::Regular,
        true,
    );
    assert_eq!(refund_regular.to_plain_string(), "22.00");
}

/// The interest-recalculation branch of `getInterestTillChargeOffForPeriod` and the non-charge-off refund formula,
/// against golden engine values (100 @ 7% DAYS_360/30): with the charge-off on Feb 15, the past period contributes its
/// full 0.58 and the containing period the engine's till-date 0.24 (later periods zero) → 0.82 — the same golden
/// quantity as `get_sum_of_due_interests_on_date(Feb 15)`. The refund formula yields
/// `2.05 (golden sum at maturity) − prior − service-computed after`, reversing at zero.
#[test]
fn interest_till_charge_off_recalculation_and_refund_formula() {
    let model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("7.0"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let mut rows: Vec<AdvancedInstallment> = six_monthly()
        .iter()
        .map(|&(from, due)| installment(from, due, "0", "0"))
        .collect();
    update_installments_by_repayment_periods(&model, &mut rows);

    let feb15 = d(2024, Month::February, 15);
    let till = interest_till_charge_off(Some(&model), &rows, feb15, ChargeOffBehaviour::Regular);
    assert_eq!(
        till.to_plain_string(),
        "0.82",
        "0.58 past + 0.24 engine till-date"
    );

    let jul1 = d(2024, Month::July, 1);
    let out = interest_refund_amount_without_charge_off(&model, jul1, &m("0"), &m("1.50"));
    assert_eq!(out.amount.to_plain_string(), "0.55", "2.05 − 0 − 1.50");
    assert!(!out.reversed);
    let out = interest_refund_amount_without_charge_off(&model, jul1, &m("0"), &m("2.05"));
    assert_eq!(out.amount.to_plain_string(), "0.00");
    assert!(out.reversed);
}

/// `handleContractTermination` (src 587) → `handleAccelerateMaturityDate` (src 2141), fee-free non-recalculation —
/// verified against the captured Java golden (`fcCaptureContractTermination`): past 100/10 (due Mar 1), current
/// 100/12 (Mar 1–Apr 1), future 100/8 (Apr 1–May 1), terminated on Mar 16. The current row's interest prorates to
/// 5.81, its due date snaps to Mar 16, the future principal folds in (→ 200) and the future row is dropped; the
/// transaction is stamped with the outstanding sweep P 300.00 / I 15.81 and the loan is contract-terminated. A
/// fully-paid schedule reverses instead.
#[test]
fn contract_termination_matches_capture() {
    let mar16 = d(2023, Month::March, 16);
    let may1 = d(2023, Month::May, 1);
    let mut rows = vec![
        installment(
            d(2023, Month::February, 1),
            d(2023, Month::March, 1),
            "100",
            "10",
        ),
        installment(
            d(2023, Month::March, 1),
            d(2023, Month::April, 1),
            "100",
            "12",
        ),
        installment(d(2023, Month::April, 1), d(2023, Month::May, 1), "100", "8"),
    ];

    let out = handle_contract_termination(None, &mut rows, mar16, may1);

    assert_eq!(out.principal.to_plain_string(), "300.00");
    assert_eq!(
        out.interest.to_plain_string(),
        "15.81",
        "10.00 past + 5.81 prorated"
    );
    assert!(!out.reversed);
    assert!(out.contract_terminated);

    // Schedule truncated: the future row is gone; the current row carries the folded principal + prorated interest
    // with its due date snapped to the termination date.
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].installment().due_date, mar16);
    assert_eq!(
        rows[1].installment().principal().to_plain_string(),
        "200.00"
    );
    assert_eq!(
        rows[1].installment().interest_charged().to_plain_string(),
        "5.81"
    );

    // Fully-paid schedule → all-zero sweep → reverse + lift.
    let mut paid = vec![installment(
        d(2023, Month::February, 1),
        d(2023, Month::March, 1),
        "100",
        "0",
    )];
    let _ = paid[0].installment_mut().pay_principal(&m("100"));
    let out = handle_contract_termination(None, &mut paid, mar16, may1);
    assert!(out.reversed);
    assert!(!out.contract_terminated);
    assert_eq!(out.principal.to_plain_string(), "0.00");
}

/// `handleCapitalizedIncomeWithEMICalculator` (src 1753) core, replaying the `fcCaptureCapitalizedIncome` engine
/// golden through the processor: 100 @ 9.4822% (fresh EMI-17.13 schedule synced onto rows), capitalized income 50 on
/// Feb 15. The engine re-amortizes to the 27.33 schedule, and the positional refresh overwrites the rows due on/after
/// the transaction date (26.46/0.87 … 27.13/0.21) while the already-due first row keeps its billed 16.34/0.79.
#[test]
fn progressive_capitalized_income_refreshes_future_rows() {
    let mut model = new_declining_balance(
        &six_monthly(),
        &bd("100"),
        &bd("9.4822"),
        1,
        2,
        RoundingMode::HalfEven,
    );
    let mut rows: Vec<AdvancedInstallment> = six_monthly()
        .iter()
        .map(|&(from, due)| installment(from, due, "0", "0"))
        .collect();
    update_installments_by_repayment_periods(&model, &mut rows);
    assert_eq!(rows[0].installment().principal().to_plain_string(), "16.34");

    handle_capitalized_income_progressive(
        &mut model,
        &mut rows,
        d(2024, Month::February, 15),
        &m("50"),
    );

    // The already-due first row keeps its billed amounts.
    assert_eq!(rows[0].installment().principal().to_plain_string(), "16.34");
    assert_eq!(
        rows[0].installment().interest_charged().to_plain_string(),
        "0.79"
    );
    // The rows due on/after Feb 15 carry the re-amortized engine schedule (the FCCI golden).
    let expected = [
        ("26.46", "0.87"),
        ("26.48", "0.85"),
        ("26.69", "0.64"),
        ("26.90", "0.43"),
        ("27.13", "0.21"),
    ];
    for (j, (principal, interest)) in expected.iter().enumerate() {
        let inst = rows[j + 1].installment();
        assert_eq!(
            inst.principal().to_plain_string(),
            *principal,
            "row {} principal",
            j + 1
        );
        assert_eq!(
            inst.interest_charged().to_plain_string(),
            *interest,
            "row {} interest",
            j + 1
        );
    }
}
