//! Progressive interest-schedule **model** + declining-balance generation — the index-based port of
//! `ProgressiveLoanInterestScheduleModel` + `RepaymentPeriod` + the `generate` / `addDisbursement` /
//! `calculateEMIValueAndRateFactorsForDecliningBalanceInterestMethod` path of `ProgressiveEMICalculator`.
//!
//! The original is a `previous`-linked mutable graph with `Memo` caching; this is the faithful behavioral port as
//! periods-in-a-`Vec` (previous-by-index, no memoization — `Memo` only caches). This slice covers the **fresh
//! declining-balance disbursement** (no payments / credits / pauses): rate factors per period, the annuity EMI, and the
//! per-period due-interest / due-principal / outstanding distribution, with the last period closing the balance. The
//! incremental operations (`payPrincipal`, `addBalanceCorrection`, re-age, …) build on this model in later slices.

use crate::interest_period::calculated_due_interest_declining;
use crate::math::Mc;
use crate::rate_factor::{fn_value, rate_factor_by_repayment_every_month};
use bigdecimal::BigDecimal;
use fc_money::Money;
use time::Date;

/// One repayment period of the schedule model (the amounts at currency scale).
#[derive(Clone, Debug)]
pub struct RepaymentPeriod {
    pub from: Date,
    pub due: Date,
    /// The (equal) installment value (`getEmi`).
    pub emi: Money,
    pub due_principal: Money,
    pub due_interest: Money,
    /// Outstanding balance after this period's principal (`getOutstandingLoanBalance`).
    pub outstanding_balance: Money,
}

/// Day-count configuration (mirrors `DaysInYearType` / `DaysInMonthType` for the month-based rate factor).
#[derive(Clone, Copy, Debug)]
pub struct DayCount {
    pub days_in_year: i64,
    pub days_in_month: i64,
    /// Days used as the period length in the rate factor (e.g. 30 for `DAYS_30`).
    pub days_in_period: i64,
    /// `repayEvery` — the repayment-period multiplier (1 = monthly, 2 = every two months, …).
    pub repay_every: i64,
}

/// The interest-schedule model.
#[derive(Clone, Debug)]
pub struct Model {
    pub periods: Vec<RepaymentPeriod>,
}

/// `PeriodDueDetails` — the due amounts of one repayment period (`getDueAmounts`).
#[derive(Clone, Debug)]
pub struct PeriodDueDetails {
    pub emi: Money,
    pub due_principal: Money,
    pub due_interest: Money,
}

impl Model {
    /// `getLoanTermInDays()` — actual days from the first period's start to the last period's due date.
    #[must_use]
    pub fn loan_term_in_days(&self) -> i64 {
        match (self.periods.first(), self.periods.last()) {
            (Some(f), Some(l)) => (l.due - f.from).whole_days(),
            _ => 0,
        }
    }

    /// `findRepaymentPeriodByFromAndDueDate` — the period with the given boundaries.
    #[must_use]
    pub fn find_period(&self, from: Date, due: Date) -> Option<&RepaymentPeriod> {
        self.periods.iter().find(|p| p.from == from && p.due == due)
    }

    /// `getDueAmounts(from, due, targetDate)` for the horizontal / no-recalculation case (target date at or after the
    /// period start): the period's EMI, due principal, and due interest. The overdue/partial `recalculateScheduleModel
    /// TillDate` + EMI-adjustment + vertical capping paths are later slices.
    #[must_use]
    pub fn due_amounts(&self, from: Date, due: Date) -> Option<PeriodDueDetails> {
        self.find_period(from, due).map(|p| PeriodDueDetails {
            emi: p.emi.clone(),
            due_principal: p.due_principal.clone(),
            due_interest: p.due_interest.clone(),
        })
    }
}

/// `generatePeriodInterestScheduleModel` + `addDisbursement` for the fresh declining-balance case: build the model from
/// the repayment-period date boundaries and a single disbursement, computing the annuity EMI and the per-period
/// due-interest / due-principal / outstanding distribution.
#[must_use]
pub fn generate_declining_balance(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    day_count: DayCount,
    scale: i64,
    mode: bigdecimal::RoundingMode,
) -> Model {
    let mc = Mc::new(12, mode);
    let money = |v: &BigDecimal| Money::of(scale, mode, v);
    let rate = mc.div(nominal_rate_pct, &BigDecimal::from(100));

    let one = BigDecimal::from(1);
    let dim = BigDecimal::from(day_count.days_in_month);
    let diy = BigDecimal::from(day_count.days_in_year);
    let dip = BigDecimal::from(day_count.days_in_period);
    let repay_every = BigDecimal::from(day_count.repay_every);

    // Per-period rate factor (month-based), and (1 + rate factor) rounded to the MathContext (`.add(ONE, mc)`).
    let rate_factors: Vec<BigDecimal> = period_dates
        .iter()
        .map(|_| {
            rate_factor_by_repayment_every_month(&mc, &rate, &repay_every, &dim, &diy, &dip, &dip)
        })
        .collect();
    let rfp1: Vec<BigDecimal> = rate_factors.iter().map(|rf| mc.add(rf, &one)).collect();

    // Annuity EMI = rateFactorN × outstanding / fnResult, at currency scale.
    let rate_factor_n = rfp1.iter().fold(one.clone(), |acc, v| mc.mul(&acc, v));
    let fn_result = rfp1
        .iter()
        .skip(1)
        .fold(one.clone(), |f, v| fn_value(&mc, &f, v));
    let emi = if rfp1.is_empty() {
        money(&BigDecimal::from(0))
    } else {
        money(&mc.div(&mc.mul(&rate_factor_n, disbursed), &fn_result))
    };

    // Distribute: declining-balance interest on the outstanding balance; principal = emi − interest; the last period
    // closes the remaining balance.
    let n = period_dates.len();
    let mut periods = Vec::with_capacity(n);
    let mut outstanding = money(disbursed);
    for (i, (from, due)) in period_dates.iter().enumerate() {
        let calc_interest = calculated_due_interest_declining(
            &mc,
            outstanding.amount(),
            &rate_factors[i],
            day_count.days_in_period,
            day_count.days_in_period,
            &BigDecimal::from(0),
        );
        let interest = money(&calc_interest);
        let last = i == n - 1;
        let (principal, emi_i) = if last {
            (outstanding.clone(), outstanding.plus(&interest))
        } else {
            let p = emi.minus(&interest);
            (p, emi.clone())
        };
        outstanding = outstanding.minus(&principal);
        periods.push(RepaymentPeriod {
            from: *from,
            due: *due,
            emi: emi_i,
            due_principal: principal,
            due_interest: interest,
            outstanding_balance: outstanding.clone(),
        });
    }

    Model { periods }
}
