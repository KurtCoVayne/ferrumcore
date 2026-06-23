//! Progressive interest-schedule **model** + declining-balance generation — the index-based port of
//! `ProgressiveLoanInterestScheduleModel` + `RepaymentPeriod` + the `generate` / `addDisbursement` /
//! `calculateEMIValueAndRateFactorsForDecliningBalanceInterestMethod` path of `ProgressiveEMICalculator`.
//!
//! The original is a `previous`-linked mutable graph with `Memo` caching; this is the faithful behavioral port as
//! periods-in-a-`Vec` (previous-by-index, no memoization). This slice covers the **fresh declining-balance
//! disbursement** (no payments / credits / pauses) for both DAYS_360/DAYS_30 (incl. repay-every-N) and ACTUAL/ACTUAL
//! (with the year-boundary partial-period split). Per-period rate factors, the annuity EMI, and the per-period
//! due-interest / due-principal / outstanding distribution with the last period closing the balance.
//!
//! Faithful details proven against `ProgressiveEMICalculatorTest`: `rateFactorPlus1` is an **exact** add
//! (`1 + ΣrateFactor`, no MathContext rounding); the EMI uses `stripTrailingZeros(rateFactorN)` /
//! `stripTrailingZeros(fnResult)`; ACTUAL/ACTUAL crossing periods split at **Dec 31** (`isInterestRecognitionOn
//! DisbursementDate == false`) summing `daysInSegment / Year.length()`.

use crate::interest_period::calculated_due_interest_declining;
use crate::math::Mc;
use crate::rate_factor::{fn_value, rate_factor_by_repayment_period};
use bigdecimal::BigDecimal;
use fc_money::Money;
use fc_money::decimal;
use time::{Date, Month};

/// One interest period within a repayment period (`InterestPeriod`). A disbursement on a period's start date produces a
/// zero-length leading stub (rate factor 0).
#[derive(Clone, Debug)]
pub struct InterestPeriod {
    pub from: Date,
    pub due: Date,
    /// `getRateFactor()` — the period rate factor, fixed to 12 decimals (zero for the disbursement stub).
    pub rate_factor: BigDecimal,
    /// `getCalculatedDueInterest()` — the interest accrued over this interest period (full precision, not currency
    /// scale).
    pub calculated_due_interest: BigDecimal,
}

/// One repayment period of the schedule model (the amounts at currency scale).
#[derive(Clone, Debug)]
pub struct RepaymentPeriod {
    pub from: Date,
    pub due: Date,
    /// The interest periods making up this repayment period (`getInterestPeriods`).
    pub interest_periods: Vec<InterestPeriod>,
    /// The installment value (`getEmi`).
    pub emi: Money,
    pub due_principal: Money,
    pub due_interest: Money,
    /// Outstanding balance after this period's principal (`getOutstandingLoanBalance`).
    pub outstanding_balance: Money,
}

/// Day-count convention for the rate factor (mirrors `DaysInYearType` / `DaysInMonthType`).
#[derive(Clone, Copy, Debug)]
pub enum DayCountKind {
    /// DAYS_360 / DAYS_30 with the repay-every multiplier in months.
    Days360Days30 { repay_every: i64 },
    /// ACTUAL days-in-month with a **fixed** days-in-year (e.g. DAYS_364, DAYS_365): `rate × actualDays / daysInYear`,
    /// no year-boundary split. Covers weekly/daily/15-day frequencies under a fixed year.
    ActualFixedYear { days_in_year: i64 },
    /// ACTUAL / ACTUAL. The year-boundary split is at **Dec 31** normally, or at **Jan 1** of the next year when
    /// `recognition_on_disbursement` is set (`isInterestRecognitionOnDisbursementDate`).
    ActualActual { recognition_on_disbursement: bool },
    /// ACTUAL days-in-month + ACTUAL days-in-year with the `FEB_29_PERIOD_ONLY` custom strategy: the days-in-year is
    /// 366 only for a period whose `(from, due]` range contains a Feb 29, else 365. The year-boundary partial split
    /// (Dec 31) is applied **only** when a year-crossing period also contains a Feb 29 (otherwise the simple
    /// `rate × actualDays / daysInYear` form is used even across a year boundary).
    ActualFeb29PeriodOnly,
    /// SAME_AS_REPAYMENT_PERIOD, monthly: `rate × repayEvery / 12` (independent of actual day counts).
    SameAsRepaymentMonths { repay_every: i64 },
}

/// `PeriodDueDetails` — the due amounts of one repayment period (`getDueAmounts`).
#[derive(Clone, Debug)]
pub struct PeriodDueDetails {
    pub emi: Money,
    pub due_principal: Money,
    pub due_interest: Money,
}

/// The interest-schedule model.
#[derive(Clone, Debug)]
pub struct Model {
    pub periods: Vec<RepaymentPeriod>,
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

    /// `getDueAmounts(from, due, targetDate)` for the horizontal / no-recalculation case: the period's EMI, due
    /// principal, and due interest.
    #[must_use]
    pub fn due_amounts(&self, from: Date, due: Date) -> Option<PeriodDueDetails> {
        self.find_period(from, due).map(|p| PeriodDueDetails {
            emi: p.emi.clone(),
            due_principal: p.due_principal.clone(),
            due_interest: p.due_interest.clone(),
        })
    }
}

/// `Year.length()` — 366 in leap years, else 365.
pub(crate) fn year_len(year: i32) -> i64 {
    if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
        366
    } else {
        365
    }
}

/// `isPeriodContainsFeb29` (src line 1330): true when some leap year in `[from.year, due.year]` has its Feb 29
/// in the half-open range `(from, due]` (`isDateInRangeFromExclusiveToInclusive`).
fn period_contains_feb29(from: Date, due: Date) -> bool {
    (from.year()..=due.year()).any(|year| {
        year_len(year) == 366
            && Date::from_calendar_date(year, Month::February, 29)
                .is_ok_and(|leap_day| leap_day > from && leap_day <= due)
    })
}

fn dec_31(year: i32, fallback: Date) -> Date {
    Date::from_calendar_date(year, Month::December, 31).unwrap_or(fallback)
}
fn jan_1(year: i32, fallback: Date) -> Date {
    Date::from_calendar_date(year, Month::January, 1).unwrap_or(fallback)
}

/// `calculatePeriodFractions` (src line 1550): `Σ_year (daysInSegment / Year.length(year))`. The end-of-year split is
/// `getFractionPeriodDueDateForEndOfYear`: Jan 1 of the next year when `recognition_on_disbursement`, else Dec 31.
pub(crate) fn calculate_period_fractions(
    mc: &Mc,
    from: Date,
    due: Date,
    recognition_on_disbursement: bool,
) -> BigDecimal {
    let mut cumulated = BigDecimal::from(0);
    let mut actual_year = from.year();
    let end_year = due.year();
    let mut actual_date = from;
    while actual_year <= end_year {
        let fraction_due = if actual_year == end_year {
            due
        } else if recognition_on_disbursement {
            jan_1(actual_year + 1, due)
        } else {
            dec_31(actual_year, due)
        };
        let ndy = BigDecimal::from(year_len(actual_year));
        let days = BigDecimal::from((fraction_due - actual_date).whole_days());
        cumulated = mc.add(&cumulated, &mc.div(&days, &ndy));
        actual_date = fraction_due;
        actual_year += 1;
    }
    cumulated
}

/// Per-period rate factor (`calculateRateFactorPerPeriodForInterest`, declining balance).
fn period_rate_factor(
    mc: &Mc,
    rate: &BigDecimal,
    from: Date,
    due: Date,
    kind: DayCountKind,
) -> BigDecimal {
    let one = BigDecimal::from(1);
    match kind {
        DayCountKind::Days360Days30 { repay_every } => rate_factor_by_repayment_period(
            mc,
            rate,
            &BigDecimal::from(30),
            &BigDecimal::from(repay_every),
            &BigDecimal::from(360),
            &one,
            &one,
        ),
        DayCountKind::ActualFixedYear { days_in_year } => {
            let days = (due - from).whole_days();
            rate_factor_by_repayment_period(
                mc,
                rate,
                &BigDecimal::from(days),
                &one,
                &BigDecimal::from(days_in_year),
                &one,
                &one,
            )
        }
        DayCountKind::SameAsRepaymentMonths { repay_every } => rate_factor_by_repayment_period(
            mc,
            rate,
            &one,
            &BigDecimal::from(repay_every),
            &BigDecimal::from(12),
            &one,
            &one,
        ),
        DayCountKind::ActualFeb29PeriodOnly => {
            let contains_feb29 = period_contains_feb29(from, due);
            // daysInYear = ACTUAL.getNumberOfDays(from) overridden to 365 when 366 but the period has no Feb 29.
            let days_in_year = if year_len(from.year()) == 366 && !contains_feb29 {
                365
            } else {
                year_len(from.year())
            };
            // Partial (Dec-31 split) only when crossing a year boundary AND the period contains a Feb 29.
            if due.year() > from.year() && contains_feb29 {
                let cpf = calculate_period_fractions(mc, from, due, false);
                decimal::set_scale(&mc.mul(rate, &cpf), i64::from(mc.precision), mc.mode)
            } else {
                let days = (due - from).whole_days();
                rate_factor_by_repayment_period(
                    mc,
                    rate,
                    &BigDecimal::from(days),
                    &one,
                    &BigDecimal::from(days_in_year),
                    &one,
                    &one,
                )
            }
        }
        DayCountKind::ActualActual {
            recognition_on_disbursement,
        } => {
            if due.year() > from.year() {
                // Partial period: rate × cumulatedFractions, fixed to precision decimals.
                let cpf = calculate_period_fractions(mc, from, due, recognition_on_disbursement);
                decimal::set_scale(&mc.mul(rate, &cpf), i64::from(mc.precision), mc.mode)
            } else {
                let days = (due - from).whole_days();
                rate_factor_by_repayment_period(
                    mc,
                    rate,
                    &BigDecimal::from(days),
                    &one,
                    &BigDecimal::from(year_len(from.year())),
                    &one,
                    &one,
                )
            }
        }
    }
}

/// `generatePeriodInterestScheduleModel` + `addDisbursement` for the fresh declining-balance case (no principal grace).
#[must_use]
pub fn generate_declining_balance(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    day_count: DayCountKind,
    scale: i64,
    mode: bigdecimal::RoundingMode,
) -> Model {
    generate_declining_balance_grace(
        period_dates,
        disbursed,
        nominal_rate_pct,
        day_count,
        scale,
        mode,
        0,
    )
}

/// As [`generate_declining_balance`], with `grace_on_principal` leading interest-only periods (no principal); the
/// annuity is computed over the remaining periods on the full balance, which stays put during grace.
#[must_use]
pub fn generate_declining_balance_grace(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    day_count: DayCountKind,
    scale: i64,
    mode: bigdecimal::RoundingMode,
    grace_on_principal: usize,
) -> Model {
    let mc = Mc::new(12, mode);
    let money = |v: &BigDecimal| Money::of(scale, mode, v);
    let rate = mc.div(nominal_rate_pct, &BigDecimal::from(100));
    let one = BigDecimal::from(1);
    let n = period_dates.len();
    let grace = grace_on_principal.min(n);

    let rate_factors: Vec<BigDecimal> = period_dates
        .iter()
        .map(|(from, due)| period_rate_factor(&mc, &rate, *from, *due, day_count))
        .collect();
    // rateFactorPlus1 is an EXACT add (no MathContext rounding).
    let rfp1: Vec<BigDecimal> = rate_factors.iter().map(|rf| rf + &one).collect();

    // Annuity EMI over the post-grace periods: stripTrailingZeros(rateFactorN) × balance / stripTrailingZeros(fnResult).
    let rate_factor_n = rfp1
        .iter()
        .skip(grace)
        .fold(one.clone(), |acc, v| mc.mul(&acc, v))
        .normalized();
    let fn_result = rfp1
        .iter()
        .skip(grace + 1)
        .fold(one.clone(), |f, v| fn_value(&mc, &f, v))
        .normalized();
    let emi = if grace >= n {
        money(&BigDecimal::from(0))
    } else {
        money(&mc.div(&mc.mul(&rate_factor_n, disbursed), &fn_result))
    };

    let mut periods = Vec::with_capacity(n);
    let mut outstanding = money(disbursed);
    for (i, (from, due)) in period_dates.iter().enumerate() {
        let days = (*due - *from).whole_days();
        let calc_interest = calculated_due_interest_declining(
            &mc,
            outstanding.amount(),
            &rate_factors[i],
            days,
            days,
            &BigDecimal::from(0),
        );
        let interest = money(&calc_interest);

        // Interest periods: the first repayment period carries a zero-length disbursement stub at its start.
        let mut interest_periods = Vec::new();
        if i == 0 {
            interest_periods.push(InterestPeriod {
                from: *from,
                due: *from,
                rate_factor: BigDecimal::from(0),
                calculated_due_interest: BigDecimal::from(0),
            });
        }
        interest_periods.push(InterestPeriod {
            from: *from,
            due: *due,
            rate_factor: rate_factors[i].clone(),
            calculated_due_interest: calc_interest,
        });

        let last = i == n - 1;
        let (principal, emi_i) = if i < grace {
            // Interest-only grace period: no principal, EMI equals the interest, balance unchanged.
            (money(&BigDecimal::from(0)), interest.clone())
        } else if last {
            (outstanding.clone(), outstanding.plus(&interest))
        } else {
            (emi.minus(&interest), emi.clone())
        };
        outstanding = outstanding.minus(&principal);
        periods.push(RepaymentPeriod {
            from: *from,
            due: *due,
            interest_periods,
            emi: emi_i,
            due_principal: principal,
            due_interest: interest,
            outstanding_balance: outstanding.clone(),
        });
    }

    Model { periods }
}

/// `generate` + `addDisbursement` for the **flat** interest method (single fresh disbursement): interest accrues on the
/// *original* disbursed amount each period (`baseAmount = total disbursed`), the EMI is `(principal + Σinterest) / n`,
/// and the last period carries the rounding remainder. Mirrors
/// `calculateEMIOnActualModelWithFlatInterestMethod` for the fresh case.
#[must_use]
pub fn generate_flat(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    day_count: DayCountKind,
    scale: i64,
    mode: bigdecimal::RoundingMode,
) -> Model {
    let mc = Mc::new(12, mode);
    let money = |v: &BigDecimal| Money::of(scale, mode, v);
    let rate = mc.div(nominal_rate_pct, &BigDecimal::from(100));
    let n = period_dates.len();

    let rate_factors: Vec<BigDecimal> = period_dates
        .iter()
        .map(|(from, due)| period_rate_factor(&mc, &rate, *from, *due, day_count))
        .collect();
    // Flat interest: on the original disbursed amount each period.
    let interests: Vec<Money> = rate_factors
        .iter()
        .map(|rf| money(&(disbursed * rf)))
        .collect();
    let total = interests
        .iter()
        .fold(money(disbursed), |acc, i| acc.plus(i));
    let period_emi = if n == 0 {
        money(&BigDecimal::from(0))
    } else {
        money(&mc.div(total.amount(), &BigDecimal::from(n as i64)))
    };
    let remainder = total.minus(&period_emi.multiplied_by(&BigDecimal::from(n as i64)));

    let mut periods = Vec::with_capacity(n);
    let mut outstanding = money(disbursed);
    for (i, (from, due)) in period_dates.iter().enumerate() {
        let interest = interests[i].clone();
        let last = i == n - 1;
        let emi_i = if last {
            period_emi.plus(&remainder)
        } else {
            period_emi.clone()
        };
        let principal = emi_i.minus(&interest);
        outstanding = outstanding.minus(&principal);

        let mut interest_periods = Vec::new();
        if i == 0 {
            interest_periods.push(InterestPeriod {
                from: *from,
                due: *from,
                rate_factor: BigDecimal::from(0),
                calculated_due_interest: BigDecimal::from(0),
            });
        }
        interest_periods.push(InterestPeriod {
            from: *from,
            due: *due,
            rate_factor: rate_factors[i].clone(),
            calculated_due_interest: disbursed * &rate_factors[i],
        });

        periods.push(RepaymentPeriod {
            from: *from,
            due: *due,
            interest_periods,
            emi: emi_i,
            due_principal: principal,
            due_interest: interest,
            outstanding_balance: outstanding.clone(),
        });
    }
    Model { periods }
}
