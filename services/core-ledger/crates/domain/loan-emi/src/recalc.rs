//! Stateful **recalculation engine** — faithful port of the mutable `ProgressiveLoanInterestScheduleModel` /
//! `RepaymentPeriod` / `InterestPeriod` graph plus the `ProgressiveEMICalculator` operations that mutate it
//! (`addDisbursement`, `addBalanceCorrection`, `payPrincipal`, `payInterest`, and the recalculation helpers
//! `calculateRateFactorForRepaymentPeriod` / `calculateOutstandingBalance` /
//! `calculateEMIOnActualModelWithDecliningBalanceInterestMethod` / `calculateLastUnpaidRepaymentPeriodEMI`).
//!
//! The original is a `previous`-linked mutable graph with `Memo` lazy caching. This is the behavioral port as
//! periods-in-a-`Vec` (previous-by-index, no memoization — the getters recompute, which is what the `Memo` cached).
//! Interest-period outstanding balances are **stored** fields (set by [`Model::calculate_outstanding_balance`], mirroring
//! `InterestPeriod.updateOutstandingLoanBalance`); the due-interest / due-principal / outstanding **getters** are
//! recomputed live from that stored state exactly like the Java getters, including the `previous.getUnrecognizedInterest`
//! carry.
//!
//! This slice covers the **declining-balance, DAYS_360 / DAYS_30** case (the `test_payoff_*` family): a single
//! disbursement, balance corrections from principal payments, interest-period splitting at the transaction date, and the
//! EMI recalculation on the unpaid tail. Other day counts / interest methods reuse [`crate::schedule`] generation and are
//! layered on later.

use crate::interest_period::calculated_due_interest_declining;
use crate::math::Mc;
use crate::rate_factor::{fn_value, rate_factor_by_repayment_period};
use bigdecimal::{BigDecimal, RoundingMode};
use fc_money::Money;
use fc_money::decimal;
use time::Date;

/// Interest method (only the two the engine implements).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterestMethod {
    DecliningBalance,
    Flat,
}

/// Day count for the recalc rate factors.
#[derive(Clone, Copy, Debug)]
pub enum DayCount {
    /// DAYS_360 / DAYS_30 with the repay-every multiplier in months.
    Days360Days30 { repay_every: i64 },
    /// ACTUAL / ACTUAL (days-in-month ACTUAL, days-in-year = `Year.length`), with the year-boundary partial split.
    ActualActual { recognition_on_disbursement: bool },
    /// ACTUAL days-in-month with a fixed days-in-year (e.g. DAYS_365 / DAYS_364): `rate × actualDays / daysInYear`,
    /// no year-boundary split.
    ActualFixedYear { days_in_year: i64 },
    /// SAME_AS_REPAYMENT_PERIOD: `rate × repayEvery / months_in_year × actualDays / calculatedDays`
    /// (`ProgressiveEMICalculator.calculateRateFactorPerPeriod*` MONTHS branch, lines 1381 & 1514, via
    /// `rateFactorByRepaymentPeriod(rate, ONE, repaymentEvery, 12, actual, calc)`). `months_in_year` = 12 monthly, 52
    /// weekly. A mid-period segment is prorated by its day-share (`actualDays/calcDays`) — the partial factor is the
    /// same formula with `actualDays` = the segment length; there is no separate partial-period formula for same-year
    /// monthly periods.
    SameAsRepayment {
        repay_every: i64,
        months_in_year: i64,
    },
    /// ACTUAL/ACTUAL with the `FEB_29_PERIOD_ONLY` custom strategy: daysInYear is 366 only for a period whose `(from,
    /// due]` contains a Feb 29 (else 365), and the Dec-31 year-boundary split applies only when a year-crossing period
    /// also contains Feb 29 (`schedule::DayCountKind::ActualFeb29PeriodOnly`).
    ActualFeb29PeriodOnly,
}

/// One interest period (`InterestPeriod`). All money at currency scale; rate factors full precision.
#[derive(Clone, Debug)]
pub struct InterestPeriod {
    pub from: Date,
    pub due: Date,
    pub rate_factor: BigDecimal,
    pub rate_factor_till_period_due_date: BigDecimal,
    pub credited_principal: Money,
    pub credited_interest: Money,
    pub disbursement_amount: Money,
    pub balance_correction_amount: Money,
    pub capitalized_income_principal: Money,
    /// `outstandingLoanBalance` — stored, set by `updateOutstandingLoanBalance`.
    pub outstanding_loan_balance: Money,
    pub is_paused: bool,
}

/// One repayment period (`RepaymentPeriod`).
#[derive(Clone, Debug)]
pub struct RepaymentPeriod {
    pub from: Date,
    pub due: Date,
    pub interest_periods: Vec<InterestPeriod>,
    pub emi: Money,
    pub original_emi: Money,
    pub paid_principal: Money,
    pub paid_interest: Money,
    /// `futureUnrecognizedInterest` — **always zero in this port**: the deferral that sets it
    /// (`calculateUnrecognizedInterestTillDateOnScheduleModelCopyAndDefer`) was proven inert across the entire
    /// 110-test Java suite (instrumented run: zero non-zero deferrals), so leaving it at zero is bit-exact-faithful.
    /// The field participates in the getter graph so a future port of the deferral only has to start setting it.
    pub future_unrecognized_interest: Money,
    pub total_disbursed_amount: Money,
    pub total_capitalized_income_amount: Money,
    pub interest_payment_grace: bool,
    /// `isInterestMovedUpward` — **never set true in this port** (its only Java writer is the inert
    /// `futureUnrecognizedInterest` deferral above), so the `!is_interest_moved_upward` guard in the interest getters
    /// currently has a dead true-branch. Kept for graph fidelity; see the deferral note.
    pub is_interest_moved_upward: bool,
    pub is_interest_moved_downward: bool,
    pub re_aged: bool,
    pub re_aged_early_repayment_holder: bool,
    pub credited_principal_moved_due_reage: Money,
    pub credited_interest_moved_due_reage: Money,
    pub fixed_interest: Money,
}

/// `PeriodDueDetails`.
#[derive(Clone, Debug)]
pub struct PeriodDueDetails {
    pub emi: Money,
    pub due_principal: Money,
    pub due_interest: Money,
}

/// The interest-schedule model + the config needed to recompute rate factors.
///
/// Config-reachability note: `grace_on_interest`, `interest_pause_for_emi`, `allow_full_term_for_tranche`, and
/// `number_of_repayments` are set to their inert defaults by every constructor and by no operation — they are
/// reachable only via direct `pub`-field mutation (which the golden tests exercise). A production wiring that needs
/// interest grace or full-term tranches must set them explicitly before the first disbursement, exactly as the
/// goldens do. (`allow_partial_period_interest` is the one flag with a constructor path,
/// `new_empty_same_as_repayment`; `emi_recalculation_enabled` defaults on.)
#[derive(Clone, Debug)]
pub struct Model {
    pub periods: Vec<RepaymentPeriod>,
    pub mc: Mc,
    pub scale: i64,
    pub mode: RoundingMode,
    /// Base nominal interest rate as a fraction (annual % / 100).
    pub rate: BigDecimal,
    /// Interest-rate changes as `(effective_date, rate_fraction)` — the rate in force on a date is the one with the
    /// latest `effective_date ≤ date`, else [`Model::rate`] (`addInterestRate` / `getInterestRate`).
    pub interest_rates: Vec<(Date, BigDecimal)>,
    pub day_count: DayCount,
    pub interest_method: InterestMethod,
    pub emi_recalculation_enabled: bool,
    /// `isAllowPartialPeriodInterestCalculation` — when false, a disbursement is snapped to the containing repayment
    /// period's from-date (no mid-period interest-period split); when true it splits at the exact date. Only consulted
    /// for [`DayCount::SameAsRepayment`]; other day counts always split at the exact date (so default `true`).
    pub allow_partial_period_interest: bool,
    /// `graceOnInterestPayment` — the number of leading repayment periods whose interest is deferred (moved to the
    /// first post-grace period). 0 = no interest grace.
    pub grace_on_interest: i64,
    /// `INTEREST_PAUSE_FOR_EMI_CALCULATION` — set true iff `grace_on_interest > 0`; when true the annuity EMI is
    /// computed treating graced periods as zero-interest (`getRateFactorPlus1ForEmi` returns ONE for them).
    pub interest_pause_for_emi: bool,
    /// `isAllowFullTermForTranche` — when true (and `number_of_repayments > 0`), each disbursement gets its OWN
    /// full-term sub-schedule starting at the containing period; the sub-schedule's per-period EMI is ADDED onto the
    /// matching existing period (`addFullTermTrancheDisbursement` / `mergeNewScheduleModelWithExistingOne`).
    pub allow_full_term_for_tranche: bool,
    /// `numberOfRepayments` — the loan's configured installment count (used by the full-term-tranche sub-schedule).
    pub number_of_repayments: i64,
}

// --- Money helpers (mirroring MathUtil / Money) ---

fn neg_to_zero(m: &Money, scale: i64, mode: RoundingMode) -> Money {
    if m.is_less_than_zero() {
        Money::zero(scale, mode)
    } else {
        m.clone()
    }
}
fn is_less_than(a: &Money, b: &Money) -> bool {
    b.is_greater_than(a)
}
fn is_equal_to(a: &Money, b: &Money) -> bool {
    decimal::compare(a.amount(), b.amount()) == 0
}
fn money_min(a: &Money, b: &Money) -> Money {
    if is_less_than(a, b) {
        a.clone()
    } else {
        b.clone()
    }
}
fn money_max(a: &Money, b: &Money) -> Money {
    if a.is_greater_than(b) {
        a.clone()
    } else {
        b.clone()
    }
}
fn money_abs(m: &Money, scale: i64, mode: RoundingMode) -> Money {
    if m.is_less_than_zero() {
        Money::zero(scale, mode).minus(m)
    } else {
        m.clone()
    }
}

/// `EmiAdjustment` — the EMI imbalance between the last two not-fully-paid related periods, used by the cent-adjust loop.
#[derive(Clone, Debug)]
struct EmiAdj {
    original_emi: Money,
    emi_difference: Money,
    num_related: usize,
    uncountable: usize,
    scale: i64,
    mode: RoundingMode,
}

impl EmiAdj {
    /// `shouldBeAdjusted`: `floor(n/2) > 0 && emiDifference != 0 && |emiDifference|×100 > originalEmi.copy(floor(n/2))`.
    fn should_be_adjusted(&self) -> bool {
        let lower_half = self.num_related / 2;
        if lower_half == 0 || self.emi_difference.is_zero() {
            return false;
        }
        let lhs = money_abs(&self.emi_difference, self.scale, self.mode)
            .multiplied_by(&BigDecimal::from(100));
        let rhs = Money::of(self.scale, self.mode, &BigDecimal::from(lower_half as i64));
        lhs.is_greater_than(&rhs)
    }

    /// `adjustedEmi` = `originalEmi + emiDifference / max(1, n − uncountablePeriods)`.
    fn adjusted_emi(&self) -> Money {
        let divisor = self.num_related.saturating_sub(self.uncountable).max(1);
        let adjustment = self
            .emi_difference
            .divided_by(&BigDecimal::from(divisor as i64));
        self.original_emi.plus(&adjustment)
    }

    fn has_less_emi_difference(&self, prev: &EmiAdj) -> bool {
        is_less_than(
            &money_abs(&self.emi_difference, self.scale, self.mode),
            &money_abs(&prev.emi_difference, prev.scale, prev.mode),
        )
    }
}

impl Model {
    fn money(&self, v: &BigDecimal) -> Money {
        Money::of(self.scale, self.mode, v)
    }
    fn zero(&self) -> Money {
        Money::zero(self.scale, self.mode)
    }

    // --- rate factors ---

    /// `getInterestRate(date)` — the rate fraction in force on `date`: the latest interest-rate change with
    /// `effective_date ≤ date`, else the base [`Model::rate`].
    fn get_interest_rate(&self, date: Date) -> &BigDecimal {
        self.interest_rates
            .iter()
            .filter(|(eff, _)| *eff <= date)
            .max_by_key(|(eff, _)| *eff)
            .map_or(&self.rate, |(_, r)| r)
    }

    /// `calculateRateFactorPerPeriod` — the per-interest-period rate factor (proportional to the interest-period length
    /// within the repayment period): DAYS_360/DAYS_30 → `rate × 30 × repayEvery / 360 × ipLen / rpLen`.
    fn rate_factor_ip(
        &self,
        ip_from: Date,
        ip_due: Date,
        rp_from: Date,
        rp_due: Date,
    ) -> BigDecimal {
        let rate = self.get_interest_rate(ip_from);
        match self.day_count {
            DayCount::Days360Days30 { repay_every } => rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from(30),
                &BigDecimal::from(repay_every),
                &BigDecimal::from(360),
                &BigDecimal::from((ip_due - ip_from).whole_days()),
                &BigDecimal::from((rp_due - rp_from).whole_days()),
            ),
            DayCount::ActualActual {
                recognition_on_disbursement,
            } => self.actual_actual_rate_factor(rate, ip_from, ip_due, recognition_on_disbursement),
            DayCount::ActualFixedYear { days_in_year } => {
                let one = BigDecimal::from(1);
                rate_factor_by_repayment_period(
                    &self.mc,
                    rate,
                    &BigDecimal::from((ip_due - ip_from).whole_days()),
                    &one,
                    &BigDecimal::from(days_in_year),
                    &one,
                    &one,
                )
            }
            DayCount::SameAsRepayment {
                repay_every,
                months_in_year,
            } => rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from(1), // multiplierInDays = ONE
                &BigDecimal::from(repay_every),
                &BigDecimal::from(months_in_year), // "daysInYear" = 12 (months)
                &BigDecimal::from((ip_due - ip_from).whole_days()), // actualDaysInPeriod (segment length)
                &BigDecimal::from((rp_due - rp_from).whole_days()), // calculatedDaysInPeriod
            ),
            DayCount::ActualFeb29PeriodOnly => self.feb29_rate_factor(rate, ip_from, ip_due),
        }
    }

    /// ACTUAL/ACTUAL rate factor over `[from, due)`: `rate × actualDays / Year.length` within one year, or the
    /// cumulated year-fraction (set to precision decimals) when the interval crosses a year boundary.
    fn actual_actual_rate_factor(
        &self,
        rate: &BigDecimal,
        from: Date,
        due: Date,
        recognition: bool,
    ) -> BigDecimal {
        let one = BigDecimal::from(1);
        if due.year() > from.year() {
            let cpf = crate::schedule::calculate_period_fractions(&self.mc, from, due, recognition);
            decimal::set_scale(
                &self.mc.mul(rate, &cpf),
                i64::from(self.mc.precision),
                self.mc.mode,
            )
        } else {
            rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from((due - from).whole_days()),
                &one,
                &BigDecimal::from(crate::schedule::year_len(from.year())),
                &one,
                &one,
            )
        }
    }

    /// ACTUAL/ACTUAL with the FEB_29_PERIOD_ONLY strategy over `[from, due)` — port of
    /// `schedule::DayCountKind::ActualFeb29PeriodOnly`: daysInYear is forced to 365 when the year is a leap year but
    /// the period contains no Feb 29; the Dec-31 cross-year split applies only when the year-crossing period contains a
    /// Feb 29.
    fn feb29_rate_factor(&self, rate: &BigDecimal, from: Date, due: Date) -> BigDecimal {
        let one = BigDecimal::from(1);
        let contains_feb29 = crate::schedule::period_contains_feb29(from, due);
        let year_len = crate::schedule::year_len(from.year());
        let days_in_year = if year_len == 366 && !contains_feb29 {
            365
        } else {
            year_len
        };
        if due.year() > from.year() && contains_feb29 {
            let cpf = crate::schedule::calculate_period_fractions(&self.mc, from, due, false);
            decimal::set_scale(
                &self.mc.mul(rate, &cpf),
                i64::from(self.mc.precision),
                self.mc.mode,
            )
        } else {
            rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from((due - from).whole_days()),
                &one,
                &BigDecimal::from(days_in_year),
                &one,
                &one,
            )
        }
    }

    /// `calculateRateFactorPerPeriodForInterest(ip.from, rp.due)` — the rate factor used for interest accrual
    /// (`rateFactorTillPeriodDueDate`): DAYS_360/DAYS_30 → `rate × 30 × periodRatio / 360 × (rpDue − ipFrom) / rpLen`
    /// with `periodRatio = repayEvery` for a regular monthly schedule.
    fn rate_factor_till(&self, ip_from: Date, rp_from: Date, rp_due: Date) -> BigDecimal {
        let rate = self.get_interest_rate(ip_from);
        match self.day_count {
            DayCount::Days360Days30 { repay_every } => rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from(30),
                &self.calculate_period_ratio(rp_from, rp_due, repay_every),
                &BigDecimal::from(360),
                &BigDecimal::from((rp_due - ip_from).whole_days()),
                &BigDecimal::from((rp_due - rp_from).whole_days()),
            ),
            DayCount::ActualActual {
                recognition_on_disbursement,
            } => self.actual_actual_rate_factor(rate, ip_from, rp_due, recognition_on_disbursement),
            DayCount::ActualFixedYear { days_in_year } => {
                let one = BigDecimal::from(1);
                rate_factor_by_repayment_period(
                    &self.mc,
                    rate,
                    &BigDecimal::from((rp_due - ip_from).whole_days()),
                    &one,
                    &BigDecimal::from(days_in_year),
                    &one,
                    &one,
                )
            }
            DayCount::SameAsRepayment {
                repay_every,
                months_in_year,
            } => rate_factor_by_repayment_period(
                &self.mc,
                rate,
                &BigDecimal::from(1),
                &BigDecimal::from(repay_every),
                &BigDecimal::from(months_in_year),
                &BigDecimal::from((rp_due - ip_from).whole_days()), // till the rp due date
                &BigDecimal::from((rp_due - rp_from).whole_days()),
            ),
            DayCount::ActualFeb29PeriodOnly => self.feb29_rate_factor(rate, ip_from, rp_due),
        }
    }

    /// `calculateSeedDate` — the schedule's seed for period-ratio counting: the model start when the period sits on the
    /// regular monthly cadence, else the period's own from date.
    fn calculate_seed_date(&self, rp_from: Date, rp_due: Date, repay_every: i64) -> Date {
        let Some(start) = self.periods.first().map(|p| p.from) else {
            return rp_from;
        };
        let mut mult = 1i64;
        let mut calc = add_months(start, mult);
        while calc < rp_due {
            mult += 1;
            calc = add_months(start, mult);
        }
        if calc == rp_due && add_months(calc, -repay_every) == rp_from {
            start
        } else {
            rp_from
        }
    }

    /// `calculatePeriodRatio` (MONTHS) — the number of repayment-cadence months the period spans relative to the seed
    /// (1 for a regular monthly period; fractional/large for an extended period after a reschedule).
    fn calculate_period_ratio(&self, rp_from: Date, rp_due: Date, repay_every: i64) -> BigDecimal {
        let seed = self.calculate_seed_date(rp_from, rp_due, repay_every);
        let num_between = months_between(seed, rp_from);
        let mut mult = num_between + 1;
        let mut from = rp_from;
        while from < rp_due {
            from = add_months(seed, mult);
            if from <= rp_due {
                mult += 1;
            } else {
                let full_period_date = from;
                mult -= num_between + 1;
                let from2 = add_months(seed, mult);
                let diff_days = (rp_due - from2).whole_days();
                let full_days = (full_period_date - from2).whole_days();
                let frac = if full_days == 0 {
                    BigDecimal::from(0)
                } else {
                    self.mc
                        .div(&BigDecimal::from(diff_days), &BigDecimal::from(full_days))
                };
                return frac + BigDecimal::from(mult);
            }
        }
        BigDecimal::from(mult - num_between - 1)
    }

    /// `generateNextRepaymentDate` (no calendar): add `repay_every` months. The month-end seed clamp only differs for
    /// seed days > 28, which the supported reschedule cases don't use.
    fn generate_next_repayment_date(&self, last: Date, repay_every: i64) -> Date {
        add_months(last, repay_every)
    }

    /// `calculateRateFactorForRepaymentPeriod` — set both rate factors on every interest period of `rp_idx`.
    fn calculate_rate_factor_for_repayment_period(&mut self, rp_idx: usize) {
        let (rp_from, rp_due) = (self.periods[rp_idx].from, self.periods[rp_idx].due);
        let n = self.periods[rp_idx].interest_periods.len();
        for k in 0..n {
            let (ifrom, idue) = {
                let ip = &self.periods[rp_idx].interest_periods[k];
                (ip.from, ip.due)
            };
            let rf = self.rate_factor_ip(ifrom, idue, rp_from, rp_due);
            let rft = self.rate_factor_till(ifrom, rp_from, rp_due);
            let ip = &mut self.periods[rp_idx].interest_periods[k];
            ip.rate_factor = rf;
            ip.rate_factor_till_period_due_date = rft;
        }
    }

    fn calculate_rate_factor_for_periods(&mut self) {
        for i in 0..self.periods.len() {
            self.calculate_rate_factor_for_repayment_period(i);
        }
    }

    // --- getters (recomputed live, faithful to the Java getter graph) ---

    /// `InterestPeriod.getCalculatedDueInterest()` (declining balance).
    fn ip_calculated_due_interest(&self, rp_idx: usize, k: usize) -> BigDecimal {
        let rp = &self.periods[rp_idx];
        let ip = &rp.interest_periods[k];
        if ip.is_paused || rp.re_aged {
            return ip.credited_interest.amount().clone();
        }
        let length_till = (rp.due - ip.from).whole_days();
        let length = (ip.due - ip.from).whole_days();
        // FLAT accrues on the total disbursed (+ capitalized) up to this interest period; declining on the outstanding.
        let base = match self.interest_method {
            InterestMethod::Flat => self.total_disbursed_till(rp_idx, k),
            InterestMethod::DecliningBalance => ip.outstanding_loan_balance.clone(),
        };
        calculated_due_interest_declining(
            &self.mc,
            base.amount(),
            &ip.rate_factor_till_period_due_date,
            length,
            length_till,
            ip.credited_interest.amount(),
        )
    }

    /// `calculateTotalDisbursedAndCapitalizedIncomeAmountTillGivenPeriod(tillIp)` — total disbursed + capitalized of the
    /// period, plus the disbursement/capitalized amounts of interest periods before `till_k` (excluding zero-length stubs
    /// whose due date equals the repayment-period start).
    fn total_disbursed_till(&self, rp_idx: usize, till_k: usize) -> Money {
        let rp = &self.periods[rp_idx];
        let mut res = rp
            .total_disbursed_amount
            .plus(&rp.total_capitalized_income_amount);
        for ip in rp.interest_periods.iter().take(till_k) {
            if ip.due != rp.from {
                res = res
                    .plus(&ip.disbursement_amount)
                    .plus(&ip.capitalized_income_principal);
            }
        }
        res
    }

    /// `RepaymentPeriod.getEmiPlusCreditedAmountsPlusFutureUnrecognizedInterest()`.
    fn emi_plus_credited_plus_future(&self, i: usize) -> Money {
        let rp = &self.periods[i];
        rp.emi
            .plus(&self.total_credited_amount(i))
            .plus(&rp.future_unrecognized_interest)
    }

    /// `RepaymentPeriod.getTotalCreditedAmount()` — credited principal + interest, minus amounts moved away by re-aging.
    fn total_credited_amount(&self, i: usize) -> Money {
        self.credited_principal(i)
            .plus(&self.credited_interest(i))
            .minus(&self.periods[i].credited_interest_moved_due_reage)
            .minus(&self.periods[i].credited_principal_moved_due_reage)
    }
    fn credited_principal(&self, i: usize) -> Money {
        let acc = self.periods[i]
            .interest_periods
            .iter()
            .fold(self.zero(), |a, ip| a.plus(&ip.credited_principal));
        neg_to_zero(&acc, self.scale, self.mode)
    }
    fn credited_interest(&self, i: usize) -> Money {
        let acc = self.periods[i]
            .interest_periods
            .iter()
            .fold(self.zero(), |a, ip| a.plus(&ip.credited_interest));
        neg_to_zero(&acc, self.scale, self.mode)
    }

    /// Forward-computes `(calculatedDueInterest, dueInterest)` for period `i` in one pass from period 0, threading the
    /// previous-period unrecognized-interest carry **iteratively**. The arithmetic and evaluation order per period are
    /// identical to the recursive getter graph (Java caches the same values via `Memo`; here the recursion is simply
    /// unrolled), so results are bit-exact — but the cost is O(i·m) instead of the exponential fan-out the naive
    /// recursion had (`calculatedDueInterest(i)` → `unrecognizedInterest(i-1)` → both getters of `i-1` → …).
    fn interest_pair(&self, i: usize) -> (Money, Money) {
        let mut carry = self.zero();
        let mut calc = self.zero();
        let mut due = self.zero();
        for j in 0..=i {
            calc = self.calculated_due_interest_with_carry(j, &carry);
            due = self.due_interest_from_calc(j, &calc);
            carry = neg_to_zero(&calc.minus(&due), self.scale, self.mode);
        }
        (calc, due)
    }

    /// `RepaymentPeriod.calculateCalculatedDueInterest()` with the previous period's unrecognized interest supplied by
    /// the caller ([`Model::interest_pair`]'s running carry) instead of recursing.
    fn calculated_due_interest_with_carry(&self, i: usize, prev_unrecognized: &Money) -> Money {
        let rp = &self.periods[i];
        let mut acc = self.zero();
        if !rp.is_interest_moved_upward && !rp.is_interest_moved_downward {
            let sum = rp
                .interest_periods
                .iter()
                .enumerate()
                .fold(BigDecimal::from(0), |a, (k, _)| {
                    a + self.ip_calculated_due_interest(i, k)
                });
            acc = self.money(&sum);
        }
        acc = acc.plus(&rp.fixed_interest);
        acc = acc.plus(&rp.future_unrecognized_interest);
        if i > 0 {
            acc = acc.plus(prev_unrecognized);
        }
        neg_to_zero(&acc, self.scale, self.mode)
    }

    /// `RepaymentPeriod.getDueInterest()` with this period's `calculatedDueInterest` supplied by the caller (the
    /// inline `getCalculatedDuePrincipal` comparison reuses it — same value the recursive form recomputed).
    fn due_interest_from_calc(&self, i: usize, calc_interest: &Money) -> Money {
        let rp = &self.periods[i];
        if rp.interest_payment_grace {
            return rp.paid_interest.clone();
        }
        let calc_due_principal = neg_to_zero(
            &self.emi_plus_credited_plus_future(i).minus(calc_interest),
            self.scale,
            self.mode,
        );
        let primary = if rp.paid_principal.is_greater_than(&calc_due_principal) {
            rp.paid_interest.clone()
        } else {
            money_min(calc_interest, &self.emi_plus_credited_plus_future(i))
        };
        money_max(&primary, &rp.paid_interest)
    }

    /// `RepaymentPeriod.calculateCalculatedDueInterest()`.
    fn calculated_due_interest(&self, i: usize) -> Money {
        self.interest_pair(i).0
    }

    /// `RepaymentPeriod.getDueInterest()`. (`getCalculatedDuePrincipal` and `getUnrecognizedInterest` no longer have
    /// standalone forms — both are inlined into the forward pass, which is their only consumer.)
    fn due_interest(&self, i: usize) -> Money {
        self.interest_pair(i).1
    }

    /// `RepaymentPeriod.getDuePrincipal()`.
    fn due_principal(&self, i: usize) -> Money {
        let base = neg_to_zero(
            &self
                .emi_plus_credited_plus_future(i)
                .minus(&self.due_interest(i)),
            self.scale,
            self.mode,
        );
        money_max(&base, &self.periods[i].paid_principal)
    }

    /// `RepaymentPeriod.getOutstandingLoanBalance()`.
    fn outstanding_loan_balance(&self, i: usize) -> Money {
        let rp = &self.periods[i];
        let Some(last) = rp.interest_periods.last() else {
            return self.zero();
        };
        let v = last
            .outstanding_loan_balance
            .plus(&last.balance_correction_amount)
            .plus(&last.capitalized_income_principal)
            .plus(&last.disbursement_amount)
            .plus(&rp.paid_principal)
            .minus(&self.due_principal(i));
        neg_to_zero(&v, self.scale, self.mode)
    }

    fn outstanding_principal(&self, i: usize) -> Money {
        neg_to_zero(
            &self.due_principal(i).minus(&self.periods[i].paid_principal),
            self.scale,
            self.mode,
        )
    }

    fn total_paid_amount(&self, i: usize) -> Money {
        self.periods[i]
            .paid_principal
            .plus(&self.periods[i].paid_interest)
    }

    fn is_fully_paid(&self, i: usize) -> bool {
        is_equal_to(
            &self.emi_plus_credited_plus_future(i),
            &self.total_paid_amount(i),
        )
    }

    // --- totals ---

    fn total_due_principal(&self) -> Money {
        // getTotalDuePrincipal = Σ getCreditedAmounts (disbursement-like amounts), negativeToZero.
        let acc =
            (0..self.periods.len()).fold(self.zero(), |a, i| a.plus(&self.credited_amounts(i)));
        neg_to_zero(&acc, self.scale, self.mode)
    }
    /// `RepaymentPeriod.getCreditedAmounts()` — Σ interest-period (disbursement + credited principal + capitalized).
    fn credited_amounts(&self, i: usize) -> Money {
        self.periods[i]
            .interest_periods
            .iter()
            .fold(self.zero(), |a, ip| {
                a.plus(&ip.disbursement_amount)
                    .plus(&ip.credited_principal)
                    .plus(&ip.capitalized_income_principal)
            })
    }
    fn total_paid_principal(&self) -> Money {
        let acc = (0..self.periods.len())
            .fold(self.zero(), |a, i| a.plus(&self.periods[i].paid_principal));
        neg_to_zero(&acc, self.scale, self.mode)
    }
    fn total_due_interest(&self) -> Money {
        let acc = (0..self.periods.len()).fold(self.zero(), |a, i| a.plus(&self.due_interest(i)));
        neg_to_zero(&acc, self.scale, self.mode)
    }
    fn total_paid_interest(&self) -> Money {
        let acc = (0..self.periods.len())
            .fold(self.zero(), |a, i| a.plus(&self.periods[i].paid_interest));
        neg_to_zero(&acc, self.scale, self.mode)
    }
    fn total_credited_principal(&self) -> Money {
        let acc =
            (0..self.periods.len()).fold(self.zero(), |a, i| a.plus(&self.credited_principal(i)));
        neg_to_zero(&acc, self.scale, self.mode)
    }

    // --- outstanding balance recomputation (updateOutstandingLoanBalance) ---

    fn calculate_outstanding_balance(&mut self) {
        for i in 0..self.periods.len() {
            let m = self.periods[i].interest_periods.len();
            for k in 0..m {
                let new_obal = self.compute_ip_outstanding(i, k);
                self.periods[i].interest_periods[k].outstanding_loan_balance = new_obal;
            }
        }
    }

    /// `InterestPeriod.updateOutstandingLoanBalance()`.
    fn compute_ip_outstanding(&self, i: usize, k: usize) -> Money {
        if k == 0 {
            // first interest period of the repayment period
            if i == 0 {
                return self.zero();
            }
            let prev_rp = i - 1;
            let Some(prev_ip) = self.periods[prev_rp].interest_periods.last() else {
                return self.zero();
            };
            let v = prev_ip
                .outstanding_loan_balance
                .plus(&prev_ip.disbursement_amount)
                .plus(&prev_ip.capitalized_income_principal)
                .plus(&prev_ip.balance_correction_amount)
                .minus(&self.due_principal(prev_rp))
                .plus(&self.periods[prev_rp].paid_principal);
            neg_to_zero(&v, self.scale, self.mode)
        } else {
            let prev_ip = &self.periods[i].interest_periods[k - 1];
            let v = prev_ip
                .outstanding_loan_balance
                .plus(&prev_ip.balance_correction_amount)
                .plus(&prev_ip.capitalized_income_principal)
                .plus(&prev_ip.disbursement_amount);
            neg_to_zero(&v, self.scale, self.mode)
        }
    }

    // --- EMI calculation ---

    /// `RepaymentPeriod.getRateFactorPlus1()` = 1 + Σ interest-period rate factors (exact add).
    fn rate_factor_plus_1(&self, i: usize) -> BigDecimal {
        self.periods[i]
            .interest_periods
            .iter()
            .fold(BigDecimal::from(1), |a, ip| a + &ip.rate_factor)
    }

    /// `getRateFactorPlus1ForEmi` — returns ONE for a graced period when interest-pause-for-EMI is enabled (so the
    /// annuity EMI is computed as if graced periods accrue no interest), else the normal `rate_factor_plus_1`.
    fn rate_factor_plus_1_for_emi(&self, i: usize) -> BigDecimal {
        if self.interest_pause_for_emi && self.periods[i].interest_payment_grace {
            BigDecimal::from(1)
        } else {
            self.rate_factor_plus_1(i)
        }
    }

    /// `applyInterestMoratoriumIfRequired` — when interest-pause-for-EMI is enabled, mark the first
    /// `min(grace_on_interest, n)` repayment periods as interest-payment-grace (their interest defers forward).
    fn apply_interest_moratorium(&mut self) {
        if !self.interest_pause_for_emi {
            return;
        }
        let n = self.periods.len();
        let g = (self.grace_on_interest.max(0) as usize).min(n);
        for rp in self.periods.iter_mut().take(g) {
            rp.interest_payment_grace = true;
        }
    }

    /// `getInitialBalanceForEmiRecalculation` for period `i`.
    fn initial_balance_for_emi(&self, i: usize) -> Money {
        let initial = if i == 0 {
            self.zero()
        } else {
            self.outstanding_loan_balance(i - 1)
        };
        let total_disbursed = self.periods[i]
            .interest_periods
            .iter()
            .fold(self.zero(), |a, ip| a.plus(&ip.disbursement_amount));
        let total_capitalized = self.periods[i]
            .interest_periods
            .iter()
            .fold(self.zero(), |a, ip| {
                a.plus(&ip.capitalized_income_principal)
            });
        initial.plus(&total_disbursed).plus(&total_capitalized)
    }

    /// `calculateEMIOnActualModelWithDecliningBalanceInterestMethod` over periods `[start..]`.
    fn calculate_emi_on_actual_model(&mut self, start: usize) {
        let n = self.periods.len();
        if start >= n {
            return;
        }
        // rateFactorN = stripTrailingZeros( Π (1 + rateFactor) ) over [start..] (grace periods contribute ONE)
        let rate_factor_n = (start..n)
            .fold(BigDecimal::from(1), |acc, i| {
                self.mc.mul(&acc, &self.rate_factor_plus_1_for_emi(i))
            })
            .normalized();
        // fnResult = stripTrailingZeros( fold fnValue over [start+1..] ) (grace periods contribute ONE)
        let fn_result = (start + 1..n)
            .fold(BigDecimal::from(1), |f, i| {
                fn_value(&self.mc, &f, &self.rate_factor_plus_1_for_emi(i))
            })
            .normalized();
        let outstanding = self.initial_balance_for_emi(start);
        let emi_amt = self.mc.div(
            &self.mc.mul(&rate_factor_n, outstanding.amount()),
            &fn_result,
        );
        let emi = self.money(&emi_amt);
        for i in start..n {
            if !is_less_than(&emi, &self.total_paid_amount(i)) {
                self.periods[i].emi = emi.clone();
                self.periods[i].original_emi = emi.clone();
            }
        }
    }

    /// `calculateEMIOnActualModelWithFlatInterestMethod` over `[start..]` (the no-payment path; the paid-period
    /// reallocation branch is not yet ported). Flat interest accrues on the total disbursed; EMI = (totalDisbursed +
    /// ΣflatInterest) / n with the rounding remainder on the last period, and each interest period's outstanding updated.
    fn calculate_emi_on_actual_model_flat(&mut self, start: usize) {
        let n = self.periods.len();
        if start >= n {
            return;
        }
        let mut sum_interest = self.zero();
        for i in start..n {
            let interest = self.calculated_due_interest(i);
            sum_interest = sum_interest.plus(&interest);
            self.periods[i].emi = interest;
        }
        let last_ip = self.periods[start].interest_periods.len().saturating_sub(1);
        let total_disbursed = self.total_disbursed_till(start, last_ip);
        // already-repaid principals (the previous period's disbursed-minus-outstanding) are excluded when the related
        // periods don't start at the first period (e.g. after addRepaymentPeriods / a re-amortize from a later date).
        let already_repaid = if start > 0 {
            let prev = start - 1;
            let prev_total =
                self.total_disbursed_till(prev, self.periods[prev].interest_periods.len());
            neg_to_zero(
                &prev_total.minus(&self.outstanding_loan_balance(prev)),
                self.scale,
                self.mode,
            )
        } else {
            self.zero()
        };
        let total = total_disbursed.plus(&sum_interest).minus(&already_repaid);
        let count = (n - start) as i64;
        let period_emi = total.divided_by(&BigDecimal::from(count));
        let remainder = total.minus(&period_emi.multiplied_by(&BigDecimal::from(count)));
        // Assign the flat EMI to every period; a period that has already been paid more than its share keeps its paid
        // amount as EMI and the excess accumulates into `reallocation` (Java: per-period adjustment loop).
        let mut reallocation = self.zero();
        for i in start..n {
            let emi = if i == n - 1 {
                period_emi.plus(&remainder)
            } else {
                period_emi.clone()
            };
            let threshold = emi
                .plus(&self.total_credited_amount(i))
                .plus(&self.periods[i].future_unrecognized_interest);
            let total_paid = self.total_paid_amount(i);
            if !is_less_than(&threshold, &total_paid) {
                self.periods[i].emi = emi.clone();
                self.periods[i].original_emi = emi;
            } else {
                let adjustment = total_paid.minus(&threshold);
                reallocation = reallocation.plus(&adjustment);
                self.periods[i].emi = total_paid.clone();
                self.periods[i].original_emi = total_paid;
            }
        }
        self.calculate_outstanding_balance();
        // Redistribute the accumulated over-payment downward (Java: reversed reallocation pass), reducing later unpaid
        // periods' EMI but never below their calculated due interest or already-paid EMI.
        if reallocation.is_greater_than(&self.zero()) {
            for i in (start..n).rev() {
                if !reallocation.is_greater_than(&self.zero()) || self.is_fully_paid(i) {
                    continue;
                }
                let minimum_new_emi = neg_to_zero(
                    &self.periods[i].emi.minus(&reallocation),
                    self.scale,
                    self.mode,
                );
                let already_paid_emi = self
                    .total_paid_amount(i)
                    .minus(&self.total_credited_amount(i))
                    .minus(&self.periods[i].future_unrecognized_interest);
                let calc_due = self.calculated_due_interest(i);
                let new_emi = money_max(&calc_due, &money_max(&minimum_new_emi, &already_paid_emi));
                reallocation = reallocation.minus(&self.periods[i].emi.minus(&new_emi));
                self.periods[i].emi = new_emi;
            }
            self.calculate_outstanding_balance();
        }
    }

    /// `isEmpty()` — true when no period has a non-zero EMI (i.e. no disbursement has set the schedule yet).
    fn is_empty(&self) -> bool {
        self.periods.iter().all(|p| p.emi.is_zero())
    }

    /// `copyWithoutPaidAmounts` — a clone with paid amounts and balance corrections zeroed (disbursements / credited /
    /// capitalized retained), used to recompute a clean annuity EMI for the new-model merge.
    fn copy_without_paid_amounts(&self) -> Model {
        let mut c = self.clone();
        let zero = c.zero();
        for rp in &mut c.periods {
            rp.paid_principal = zero.clone();
            rp.paid_interest = zero.clone();
            rp.future_unrecognized_interest = zero.clone();
            for ip in &mut rp.interest_periods {
                ip.balance_correction_amount = zero.clone();
            }
        }
        c
    }

    /// `calculateEMIOnNewModelAndMerge` — recompute the clean annuity EMI on a paid-amount-free copy, then merge those
    /// EMIs back into the real periods from the related-periods' first due date onward (the 2nd+ disbursement path).
    fn calculate_emi_on_new_model_and_merge(&mut self, start: usize) {
        let n = self.periods.len();
        if start >= n {
            return;
        }
        let first_due = self.periods[start].due;
        let mut copy = self.copy_without_paid_amounts();
        copy.calculate_rate_factor_for_periods();
        copy.calculate_outstanding_balance();
        copy.calculate_emi_on_actual_model(start);
        copy.calculate_outstanding_balance();
        copy.calculate_last_unpaid_emi();
        copy.check_and_adjust_emi_if_needed(start);
        for i in 0..n {
            if self.periods[i].due >= first_due {
                self.periods[i].emi = copy.periods[i].emi.clone();
                self.periods[i].original_emi = copy.periods[i].original_emi.clone();
            }
        }
    }

    /// `calculateLastUnpaidRepaymentPeriodEMI` (without the futureUnrecognizedInterest deferral, which only applies to
    /// late/cross-period payments not exercised by the basic payoff path).
    fn calculate_last_unpaid_emi(&mut self) {
        let total_due_paid_diff = self
            .total_due_principal()
            .minus(&self.total_paid_principal());
        for i in 0..self.periods.len() {
            let outstanding_principal = self.outstanding_principal(i);
            if outstanding_principal.is_greater_than(&total_due_paid_diff) {
                let delta = outstanding_principal.minus(&total_due_paid_diff);
                let new_emi = self.periods[i].emi.minus(&delta);
                self.periods[i].emi = new_emi;
                let minimum_emi = self.periods[i]
                    .paid_interest
                    .plus(&self.periods[i].paid_principal);
                if is_less_than(&self.periods[i].emi, &minimum_emi) {
                    self.periods[i].emi = minimum_emi;
                }
            }
        }

        let last_unpaid = (0..self.periods.len()).rfind(|&i| !self.is_fully_paid(i));
        let last_unpaid = match last_unpaid {
            Some(i) => i,
            None => {
                if self.total_paid_principal().is_zero() {
                    let last = self.periods.len().checked_sub(1);
                    match last {
                        Some(i)
                            if !self.periods[i].interest_periods.is_empty()
                                && self.outstanding_loan_balance(i).is_greater_than_zero() =>
                        {
                            i
                        }
                        _ => return,
                    }
                } else {
                    return;
                }
            }
        };

        self.periods[last_unpaid].future_unrecognized_interest = self.zero();
        for rp in &mut self.periods {
            rp.is_interest_moved_upward = false;
        }

        let total_due_interest =
            (0..self.periods.len()).fold(self.zero(), |a, i| a.plus(&self.due_interest(i)));
        let total_emi = (0..self.periods.len()).fold(self.zero(), |a, i| {
            a.plus(&self.emi_plus_credited_plus_future(i))
        });
        let total_disbursed = self.periods.iter().fold(self.zero(), |a, rp| {
            rp.interest_periods
                .iter()
                .fold(a, |b, ip| b.plus(&ip.disbursement_amount))
        });
        let total_capitalized = self.periods.iter().fold(self.zero(), |a, rp| {
            rp.interest_periods
                .iter()
                .fold(a, |b, ip| b.plus(&ip.capitalized_income_principal))
        });
        let diff = total_disbursed
            .plus(&total_capitalized)
            .plus(&self.total_credited_principal())
            .plus(&total_due_interest)
            .minus(&total_emi);
        let adjusted = self.periods[last_unpaid].emi.plus(&diff);
        self.periods[last_unpaid].emi = adjusted;
        let floor = self
            .total_paid_amount(last_unpaid)
            .minus(&self.total_credited_amount(last_unpaid));
        if is_less_than(&self.periods[last_unpaid].emi, &floor) {
            self.periods[last_unpaid].emi = floor;
            self.calculate_last_unpaid_emi();
        }
    }

    // --- public getters for assertions ---

    /// `RepaymentPeriod.getEmi()`.
    #[must_use]
    pub fn period_emi(&self, i: usize) -> Money {
        self.periods[i].emi.clone()
    }
    /// `ProgressiveLoanInterestScheduleModel.getTotalDueInterest()`.
    #[must_use]
    pub fn total_due_interest_amount(&self) -> Money {
        self.total_due_interest()
    }
    /// `RepaymentPeriod.getDueInterest()`.
    #[must_use]
    pub fn period_due_interest(&self, i: usize) -> Money {
        self.due_interest(i)
    }
    /// `RepaymentPeriod.getDuePrincipal()`.
    #[must_use]
    pub fn period_due_principal(&self, i: usize) -> Money {
        self.due_principal(i)
    }
    /// `RepaymentPeriod.getOutstandingLoanBalance()`.
    #[must_use]
    pub fn period_outstanding(&self, i: usize) -> Money {
        self.outstanding_loan_balance(i)
    }
    /// `RepaymentPeriod.isFullyPaid()`.
    #[must_use]
    pub fn period_fully_paid(&self, i: usize) -> bool {
        self.is_fully_paid(i)
    }
    /// `InterestPeriod.getRateFactor()` set to precision decimals (`applyMathContext`).
    #[must_use]
    pub fn ip_rate_factor(&self, i: usize, k: usize) -> BigDecimal {
        decimal::set_scale(
            &self.periods[i].interest_periods[k].rate_factor,
            i64::from(self.mc.precision),
            self.mc.mode,
        )
    }
    /// `InterestPeriod.getCalculatedDueInterest()`.
    #[must_use]
    pub fn ip_calc_due_interest(&self, i: usize, k: usize) -> BigDecimal {
        self.ip_calculated_due_interest(i, k)
    }
    /// `RepaymentPeriod.getCreditedPrincipal()` — the chargeback-credited principal recorded on the period's interest
    /// periods (read by the processor's `updateInstallmentsByRepaymentPeriods` model→installment sync).
    #[must_use]
    pub fn period_credited_principal(&self, i: usize) -> Money {
        self.credited_principal(i)
    }
    /// `RepaymentPeriod.getCreditedInterest()`.
    #[must_use]
    pub fn period_credited_interest(&self, i: usize) -> Money {
        self.credited_interest(i)
    }

    /// `getSumOfDueInterestsOnDate(scheduleModel, subjectDate)` (ProgressiveEMICalculator line 2005): the sum over
    /// every repayment period of `getDueAmounts(from, due, subjectDate).getDueInterest()` — the total interest that
    /// would be due were the loan settled on `subject_date`. (The per-period from/due always locate their own period,
    /// so the lookup cannot miss; a missing period contributes zero, mirroring the impossible-`null` case.)
    #[must_use]
    pub fn get_sum_of_due_interests_on_date(&self, subject_date: Date) -> Money {
        let bounds: Vec<(Date, Date)> = self.periods.iter().map(|rp| (rp.from, rp.due)).collect();
        bounds.iter().fold(self.zero(), |acc, &(from, due)| {
            match self.get_due_amounts(from, due, subject_date) {
                Some(details) => acc.plus(&details.due_interest),
                None => acc,
            }
        })
    }

    // --- mutating operations ---

    /// `changeOutstandingBalanceAndUpdateInterestPeriods` — find the repayment period containing `date`, then either add
    /// the amounts to the interest period that ends on `date`, or split a new interest period at `date`. Returns the
    /// affected repayment-period index.
    fn change_outstanding_and_split(
        &mut self,
        date: Date,
        disbursed: &Money,
        correction: &Money,
        capitalized: &Money,
    ) -> Option<usize> {
        let rp_idx = self.find_period_for_balance_change(date)?;
        let is_last = rp_idx == self.periods.len() - 1;
        let is_change_on_maturity = is_last && date == self.periods[rp_idx].due;

        // findInterestPeriodForBalanceChange
        let target_k = if is_change_on_maturity {
            let last_k = self.periods[rp_idx].interest_periods.len() - 1;
            let last = &self.periods[rp_idx].interest_periods[last_k];
            if (last.due - last.from).whole_days() == 0 {
                Some(last_k)
            } else {
                None
            }
        } else {
            self.periods[rp_idx]
                .interest_periods
                .iter()
                .position(|ip| ip.due == date)
        };

        if let Some(k) = target_k {
            let ip = &mut self.periods[rp_idx].interest_periods[k];
            ip.disbursement_amount = ip.disbursement_amount.plus(disbursed);
            ip.capitalized_income_principal = ip.capitalized_income_principal.plus(capitalized);
            ip.balance_correction_amount = ip.balance_correction_amount.plus(correction);
        } else {
            self.insert_interest_period(rp_idx, date, disbursed, correction, capitalized);
        }
        Some(rp_idx)
    }

    fn find_period_for_balance_change(&self, date: Date) -> Option<usize> {
        self.periods.iter().position(|p| {
            // isInPeriod: from < date <= due, or (first period) from <= date <= due.
            let first = std::ptr::eq(p, &self.periods[0]);
            if first {
                date >= p.from && date <= p.due
            } else {
                date > p.from && date <= p.due
            }
        })
    }

    /// `insertInterestPeriod` — split the interest period preceding `date` at `date`.
    fn insert_interest_period(
        &mut self,
        rp_idx: usize,
        date: Date,
        disbursed: &Money,
        correction: &Money,
        capitalized: &Money,
    ) {
        // findPreviousInterestPeriod: last ip with date.isAfter(from) && !date.isAfter(due); else first.
        let prev_k = self.periods[rp_idx]
            .interest_periods
            .iter()
            .enumerate()
            .filter(|(_, ip)| date > ip.from && date <= ip.due)
            .map(|(k, _)| k)
            .next_back()
            .unwrap_or(0);
        let original_due = self.periods[rp_idx].interest_periods[prev_k].due;
        let prev_from = self.periods[rp_idx].interest_periods[prev_k].from;
        let new_due_date = if date < prev_from {
            prev_from
        } else if date > original_due {
            original_due
        } else {
            date
        };
        let is_paused = self.periods[rp_idx].interest_periods[prev_k].is_paused;
        {
            let prev = &mut self.periods[rp_idx].interest_periods[prev_k];
            prev.due = new_due_date;
            prev.disbursement_amount = prev.disbursement_amount.plus(disbursed);
            prev.capitalized_income_principal = prev.capitalized_income_principal.plus(capitalized);
            prev.balance_correction_amount = prev.balance_correction_amount.plus(correction);
        }
        let new_ip = InterestPeriod {
            from: new_due_date,
            due: original_due,
            rate_factor: BigDecimal::from(0),
            rate_factor_till_period_due_date: BigDecimal::from(0),
            credited_principal: self.zero(),
            credited_interest: self.zero(),
            disbursement_amount: self.zero(),
            balance_correction_amount: self.zero(),
            capitalized_income_principal: self.zero(),
            outstanding_loan_balance: self.zero(),
            is_paused,
        };
        self.periods[rp_idx]
            .interest_periods
            .insert(prev_k + 1, new_ip);
    }

    /// `addDisbursement(date, amount)` for the declining-balance, single-tranche case.
    /// `addDisbursement` lines 128-133: when SAME_AS_REPAYMENT && !allowPartial, snap a disbursement to the from-date of
    /// the first repayment period whose due date is after the disbursement date (the containing period's start), so the
    /// balance change lands on a period boundary and no mid-period split is created. Otherwise the exact date is used.
    fn effective_disbursement_date(&self, date: Date) -> Date {
        let snap = matches!(self.day_count, DayCount::SameAsRepayment { .. })
            && !self.allow_partial_period_interest;
        if snap {
            self.periods
                .iter()
                .find(|rp| rp.due > date)
                .map_or(date, |rp| rp.from)
        } else {
            date
        }
    }

    /// `addFullTermTrancheDisbursement` + `mergeNewScheduleModelWithExistingOne` (lines 155-248): generate a fresh
    /// `number_of_repayments`-period sub-schedule for this tranche starting at the period containing `date` (else
    /// maturity), then ADD each sub-schedule period's `duePrincipal + dueInterest` onto the matching existing period's
    /// EMI (matched by from+due dates), appending new periods past the original maturity when the tranche extends
    /// beyond it.
    fn add_full_term_tranche_disbursement(&mut self, date: Date, amount: &Money) {
        let repay_every = match self.day_count {
            DayCount::Days360Days30 { repay_every }
            | DayCount::SameAsRepayment { repay_every, .. } => repay_every,
            _ => 1,
        };
        // firstDisbursedPeriodStartDate: the from-date of the period containing `date` ([from, due)), else maturity.
        let start_date = self
            .periods
            .iter()
            .find(|rp| rp.from <= date && date < rp.due)
            .map(|rp| rp.from)
            .unwrap_or_else(|| self.periods.last().map_or(date, |p| p.due));

        // Temporary full-term sub-schedule for this tranche (its own `number_of_repayments` periods + fresh EMI).
        let n = self.number_of_repayments.max(0) as usize;
        let temp_periods: Vec<(Date, Date)> = (0..n)
            .map(|k| {
                (
                    add_months(start_date, k as i64 * repay_every),
                    add_months(start_date, (k as i64 + 1) * repay_every),
                )
            })
            .collect();
        let rate_pct = &self.rate * BigDecimal::from(100);
        let temp = new_declining_balance_with_day_count(
            &temp_periods,
            amount.amount(),
            &rate_pct,
            self.day_count,
            self.scale,
            self.mode,
        );

        // Apply the disbursement to the existing model's outstanding balance.
        self.change_outstanding_and_split(date, amount, &self.zero(), &self.zero());

        for k in 0..temp.periods.len() {
            let add_emi = temp.due_principal(k).plus(&temp.due_interest(k));
            let tf = temp.periods[k].from;
            let td = temp.periods[k].due;
            if let Some(idx) = self
                .periods
                .iter()
                .position(|ep| ep.from == tf && ep.due == td)
            {
                let new_emi = self.periods[idx].emi.plus(&add_emi);
                self.periods[idx].emi = new_emi.clone();
                self.periods[idx].original_emi = new_emi;
            } else {
                // Append a period beyond the original maturity (faithful port of lines 232-242): chain to the last
                // existing period, EMI = the tranche period's principal+interest, inherit the last total-disbursed.
                let zero = self.zero();
                let last = self.periods.last();
                let prev_due = last.map(|p| p.due);
                let total_disbursed = last
                    .map(|p| p.total_disbursed_amount.clone())
                    .unwrap_or_else(|| zero.clone());
                let from = if k == 0 { prev_due.unwrap_or(tf) } else { tf };
                self.periods.push(RepaymentPeriod {
                    from,
                    due: td,
                    interest_periods: vec![InterestPeriod {
                        from,
                        due: td,
                        rate_factor: BigDecimal::from(0),
                        rate_factor_till_period_due_date: BigDecimal::from(0),
                        credited_principal: zero.clone(),
                        credited_interest: zero.clone(),
                        disbursement_amount: zero.clone(),
                        balance_correction_amount: zero.clone(),
                        capitalized_income_principal: zero.clone(),
                        outstanding_loan_balance: zero.clone(),
                        is_paused: false,
                    }],
                    emi: add_emi.clone(),
                    original_emi: add_emi.clone(),
                    paid_principal: zero.clone(),
                    paid_interest: zero.clone(),
                    future_unrecognized_interest: zero.clone(),
                    total_disbursed_amount: total_disbursed,
                    total_capitalized_income_amount: zero.clone(),
                    interest_payment_grace: false,
                    is_interest_moved_upward: false,
                    is_interest_moved_downward: false,
                    re_aged: false,
                    re_aged_early_repayment_holder: false,
                    credited_principal_moved_due_reage: zero.clone(),
                    credited_interest_moved_due_reage: zero.clone(),
                    fixed_interest: zero,
                });
            }
        }
        self.calculate_rate_factor_for_periods();
        self.calculate_outstanding_balance();
        self.calculate_last_unpaid_emi();
    }

    pub fn add_disbursement(&mut self, date: Date, amount: &Money) {
        // `addDisbursement` lines 128-133: under SAME_AS_REPAYMENT with !allowPartial, snap the disbursement to the
        // containing period's from-date so no mid-period interest-period split occurs (the new principal accrues for
        // the whole period). The snap happens before everything downstream, exactly like Java's `effectiveDueDate`.
        let date = self.effective_disbursement_date(date);
        for rp in &mut self.periods {
            if date <= rp.from {
                rp.total_disbursed_amount = rp.total_disbursed_amount.plus(amount);
            }
        }
        // `addDisbursement` lines 142-144: full-term-tranche disbursements build a per-tranche full-term sub-schedule
        // and merge its EMIs onto the existing periods (instead of the single-schedule recompute below).
        if self.allow_full_term_for_tranche && self.number_of_repayments > 0 {
            self.add_full_term_tranche_disbursement(date, amount);
            return;
        }
        let was_empty = self.is_empty();
        if let Some(rp_idx) =
            self.change_outstanding_and_split(date, amount, &self.zero(), &self.zero())
        {
            self.calculate_emi_value_and_rate_factors(rp_idx, date, was_empty);
        }
    }

    /// `calculateEMIValueAndRateFactors` — the shared recompute tail of `addDisbursement` / `addCapitalizedIncome` /
    /// `changeInterestRate`: rate factors + EMI over the related periods (declining annuity with the
    /// first-vs-subsequent split and cent-adjust, or the flat EMI).
    fn calculate_emi_value_and_rate_factors(&mut self, rp_idx: usize, date: Date, was_empty: bool) {
        // calculateEMIValueAndRateFactorsForDecliningBalanceInterestMethod on the related periods.
        let related_start = self.related_start(self.effective_due_date(rp_idx, date));
        // applyInterestMoratoriumIfRequired (called from both the declining & flat EMI paths) before rate factors.
        self.apply_interest_moratorium();
        if self.interest_method == InterestMethod::Flat {
            // calculateEMIValueAndRateFactorsForFlatInterestMethod: rate factors then the flat EMI (which updates
            // outstanding internally); no annuity / new-model-merge / last-unpaid / cent-adjust.
            self.calculate_rate_factor_for_periods();
            self.calculate_emi_on_actual_model_flat(related_start);
            return;
        }
        self.calculate_rate_factor_for_periods();
        self.calculate_outstanding_balance();
        if was_empty {
            // onlyOnActualModelShouldApply (first disbursement / empty schedule).
            self.calculate_emi_on_actual_model(related_start);
        } else {
            // Subsequent operation: recompute on a clean copy and merge the EMIs back.
            self.calculate_emi_on_new_model_and_merge(related_start);
        }
        self.calculate_outstanding_balance();
        self.calculate_last_unpaid_emi();
        if was_empty {
            self.check_and_adjust_emi_if_needed(related_start);
        }
    }

    /// `addCapitalizedIncome(scheduleModel, transactionDueDate, transactionAmount)` (ProgressiveEMICalculator 269):
    /// record the capitalized income on every period starting on/after the date (`totalCapitalizedIncomeAmount`),
    /// register the amount on the interest-period timeline (the capitalized slot of
    /// `changeOutstandingBalanceAndUpdateInterestPeriods`, splitting at the date), and re-run the shared EMI
    /// recompute over the related periods — the capitalized amount joins the outstanding balance like a
    /// disbursement, without being a disbursement.
    pub fn add_capitalized_income(&mut self, date: Date, amount: &Money) {
        for rp in &mut self.periods {
            if date <= rp.from {
                rp.total_capitalized_income_amount =
                    rp.total_capitalized_income_amount.plus(amount);
            }
        }
        let was_empty = self.is_empty();
        if let Some(rp_idx) =
            self.change_outstanding_and_split(date, &self.zero(), &self.zero(), amount)
        {
            self.calculate_emi_value_and_rate_factors(rp_idx, date, was_empty);
        }
    }

    /// `addBalanceCorrection`.
    pub fn add_balance_correction(&mut self, date: Date, amount: &Money) {
        if let Some(rp_idx) =
            self.change_outstanding_and_split(date, &self.zero(), amount, &self.zero())
        {
            self.calculate_rate_factor_for_repayment_period(rp_idx);
            self.calculate_outstanding_balance();
            self.calculate_last_unpaid_emi();
        }
    }

    /// `getEffectiveRepaymentDueDate` — the due date the recalculation starts from.
    fn effective_due_date(&self, rp_idx: usize, operation_date: Date) -> Date {
        // isRelatedToNextRepaymentPeriod: changedPeriod.due == operationDate → use next period's due.
        if self.periods[rp_idx].due == operation_date && rp_idx + 1 < self.periods.len() {
            self.periods[rp_idx + 1].due
        } else {
            self.periods[rp_idx].due
        }
    }

    /// `getRelatedRepaymentPeriods(calculateFromDueDate)` → index of the first period whose due date is `>=` the given
    /// due date. (Periods before it are already settled and excluded from the EMI recompute.)
    fn related_start(&self, from_due: Date) -> usize {
        self.periods
            .iter()
            .position(|p| p.due >= from_due)
            .unwrap_or(0)
    }

    /// `payInterest`.
    pub fn pay_interest(&mut self, from: Date, due: Date, _transaction_date: Date, amount: &Money) {
        let Some(rp_idx) = self.find_repayment_period(from, due) else {
            return;
        };
        // Amend if outstanding interest is lower.
        let final_amount = if self
            .total_paid_interest()
            .plus(amount)
            .is_greater_than(&self.total_due_interest())
        {
            self.total_due_interest().minus(&self.total_paid_interest())
        } else {
            amount.clone()
        };
        self.periods[rp_idx].paid_interest = self.periods[rp_idx].paid_interest.plus(&final_amount);
        self.calculate_outstanding_balance();
        self.calculate_last_unpaid_emi();
    }

    /// `payPrincipal`.
    pub fn pay_principal(&mut self, from: Date, due: Date, transaction_date: Date, amount: &Money) {
        if amount.is_zero() {
            return;
        }
        let Some(rp_idx) = self.find_repayment_period(from, due) else {
            return;
        };
        let final_amount = if self.total_paid_principal().plus(amount).is_greater_than(
            &self
                .total_due_principal()
                .plus(&self.total_credited_principal()),
        ) {
            self.total_due_principal()
                .minus(&self.total_paid_principal())
        } else {
            amount.clone()
        };
        self.periods[rp_idx].paid_principal =
            self.periods[rp_idx].paid_principal.plus(&final_amount);
        let balance_correction_date = if due < transaction_date {
            due
        } else {
            transaction_date
        };
        let negated = self.zero().minus(amount); // principalAmount.negated()
        self.add_balance_correction(balance_correction_date, &negated);
        let not_fully_repaid = (0..self.periods.len())
            .filter(|&i| !self.is_fully_paid(i))
            .count();
        let multiple_unpaid = not_fully_repaid > 1;
        if self.emi_recalculation_enabled {
            // If a future period is (over)paid early, mark it fully paid by setting its EMI to the paid amount.
            if multiple_unpaid {
                let transaction_before = transaction_date < self.periods[rp_idx].from;
                if transaction_before {
                    let total_paid = self.total_paid_amount(rp_idx);
                    let total_credited = self.total_credited_amount(rp_idx);
                    let original_with_credit =
                        self.periods[rp_idx].original_emi.plus(&total_credited);
                    if total_paid.is_greater_than(&self.emi_plus_credited_plus_future(rp_idx))
                        || is_equal_to(&total_paid, &original_with_credit)
                    {
                        self.periods[rp_idx].emi = total_paid.minus(&total_credited);
                    }
                }
            }
            self.calculate_last_unpaid_emi();
        }
    }

    fn find_repayment_period(&self, from: Date, due: Date) -> Option<usize> {
        self.periods
            .iter()
            .position(|p| p.from == from && p.due == due)
    }

    /// `creditPrincipal` — a chargeback that re-adds principal.
    pub fn credit_principal(&mut self, transaction_date: Date, amount: &Money) {
        let z = self.zero();
        self.add_credit(transaction_date, amount, &z);
    }

    /// `creditInterest` — a chargeback that re-adds interest.
    pub fn credit_interest(&mut self, transaction_date: Date, amount: &Money) {
        let z = self.zero();
        self.add_credit(transaction_date, &z, amount);
    }

    /// `addCredit` — apply the credited principal as a positive balance correction (raising the outstanding at the
    /// transaction date) and record the credited principal / interest on the interest period starting on that date.
    fn add_credit(
        &mut self,
        transaction_date: Date,
        credited_principal: &Money,
        credited_interest: &Money,
    ) {
        let z = self.zero();
        if let Some(rp_idx) =
            self.change_outstanding_and_split(transaction_date, &z, credited_principal, &z)
        {
            self.add_credited_amounts_to_interest_period(
                transaction_date,
                credited_principal,
                credited_interest,
            );
            self.calculate_rate_factor_for_repayment_period(rp_idx);
            self.calculate_outstanding_balance();
            self.calculate_last_unpaid_emi();
        }
    }

    /// `addCreditedAmountsToInterestPeriod` — find the period whose credit range contains `transaction_date` (non-last:
    /// `from ≤ date < due`; last: `from ≤ date ≤ due`) and add the credited amounts to the (last) interest period
    /// starting on that date.
    fn add_credited_amounts_to_interest_period(
        &mut self,
        transaction_date: Date,
        credited_principal: &Money,
        credited_interest: &Money,
    ) {
        let n = self.periods.len();
        let rp = self.periods.iter().enumerate().position(|(idx, p)| {
            if idx == n - 1 {
                transaction_date >= p.from && transaction_date <= p.due
            } else {
                transaction_date >= p.from && transaction_date < p.due
            }
        });
        if let Some(rp_idx) = rp {
            let k = self.periods[rp_idx]
                .interest_periods
                .iter()
                .enumerate()
                .filter(|(_, ip)| ip.from == transaction_date)
                .map(|(k, _)| k)
                .next_back();
            if let Some(k) = k {
                let ip = &mut self.periods[rp_idx].interest_periods[k];
                ip.credited_principal = ip.credited_principal.plus(credited_principal);
                ip.credited_interest = ip.credited_interest.plus(credited_interest);
            }
        }
    }

    /// `findRepaymentPeriod(targetDate)` — the period whose `isInPeriod(date, from, due, isFirstRepaymentPeriod)` holds.
    fn find_period_containing(&self, date: Date) -> Option<usize> {
        self.periods.iter().enumerate().position(|(idx, p)| {
            if idx == 0 {
                date >= p.from && date <= p.due
            } else {
                date > p.from && date <= p.due
            }
        })
    }

    /// `RepaymentPeriod.findInterestPeriod(targetDate)` — last interest period of `rp_idx` containing `date`
    /// (`isInPeriod`, first-period-first-interest-period inclusive on the from side).
    fn find_interest_period_in(&self, rp_idx: usize, date: Date) -> Option<usize> {
        let ips = &self.periods[rp_idx].interest_periods;
        let mut found = None;
        for (k, ip) in ips.iter().enumerate() {
            let inclusive_from = rp_idx == 0 && k == 0;
            let in_period = if inclusive_from {
                date >= ip.from && date <= ip.due
            } else {
                date > ip.from && date <= ip.due
            };
            if in_period {
                found = Some(k);
            }
        }
        found
    }

    /// `recalculateScheduleModelTillDate` — a deep copy with interest periods clipped to `target` (so interest accrues
    /// only up to the target date), then rate factors / outstanding balances / tail EMI recomputed.
    fn recalculate_till_date(&self, target: Date) -> Model {
        let mut copy = self.clone();
        let first_from = copy.periods.first().map(|p| p.from);
        let last_due = copy.periods.last().map(|p| p.due);
        let before_first = first_from.is_some_and(|f| target < f);
        let after_maturity = last_due.is_some_and(|l| target >= l);
        if before_first {
            for rp in &mut copy.periods {
                rp.interest_periods.clear();
            }
            return copy;
        }
        if after_maturity {
            return copy;
        }
        let Some(target_rp) = copy.find_period_containing(target) else {
            return copy;
        };
        for i in 0..copy.periods.len() {
            if copy.periods[i].due <= target {
                continue;
            }
            if i == target_rp {
                if let Some(k) = copy.find_interest_period_in(i, target) {
                    copy.periods[i].interest_periods[k].due = target;
                    let next = k + 1;
                    if copy.periods[i].interest_periods.len() > next {
                        let (cp, ci) = {
                            let n = &copy.periods[i].interest_periods[next];
                            (n.credited_principal.clone(), n.credited_interest.clone())
                        };
                        let ip = &mut copy.periods[i].interest_periods[k];
                        ip.credited_principal = ip.credited_principal.plus(&cp);
                        ip.credited_interest = ip.credited_interest.plus(&ci);
                    }
                    copy.periods[i].interest_periods.truncate(next);
                }
            } else if i > 0
                && i - 1 == target_rp
                && copy.periods[i].interest_periods.first().is_some_and(|ip| {
                    ip.credited_interest.is_greater_than_zero()
                        || ip.credited_principal.is_greater_than_zero()
                })
            {
                let from = copy.periods[i].interest_periods[0].from;
                copy.periods[i].interest_periods[0].due = from;
                copy.periods[i].interest_periods.truncate(1);
            } else {
                copy.periods[i].interest_periods.clear();
            }
        }
        copy.calculate_rate_factor_for_periods();
        copy.calculate_outstanding_balance();
        copy.calculate_last_unpaid_emi();
        copy
    }

    fn total_outstanding_principal(&self) -> Money {
        neg_to_zero(
            &self
                .total_due_principal()
                .minus(&self.total_paid_principal()),
            self.scale,
            self.mode,
        )
    }

    /// `adjustEmiIfRequired` (horizontal) — mutate the period's EMI for the due-amount query when the target date is on
    /// or before the period's start.
    fn adjust_emi_if_required(&mut self, rp_idx: usize, target: Date, not_fully_repaid: usize) {
        if target > self.periods[rp_idx].from {
            return;
        }
        if not_fully_repaid > 1 {
            self.periods[rp_idx].emi = self.periods[rp_idx].original_emi.clone();
            let emi_adjustment = neg_to_zero(
                &self
                    .outstanding_principal(rp_idx)
                    .minus(&self.total_outstanding_principal()),
                self.scale,
                self.mode,
            );
            self.periods[rp_idx].emi = self.periods[rp_idx].emi.minus(&emi_adjustment);
            return;
        }
        if self.is_fully_paid(rp_idx) && not_fully_repaid == 1 {
            let candidate = self
                .total_due_principal()
                .minus(&self.total_paid_principal())
                .plus(&self.periods[rp_idx].paid_principal)
                .plus(&self.due_interest(rp_idx));
            self.periods[rp_idx].emi = money_min(&self.periods[rp_idx].original_emi, &candidate);
        }
    }

    /// `getEmiAdjustment(related)` — over the related periods `[start..]`, the EMI difference between the last two
    /// not-fully-paid periods (scanning from the end); falls back to a zero difference on the first period.
    fn get_emi_adjustment(&self, start: usize) -> EmiAdj {
        let n = self.periods.len();
        let num_related = n - start;
        let mut idx = n - 1;
        while idx > start {
            if !self.is_fully_paid(idx) && !self.is_fully_paid(idx - 1) {
                let original = self.periods[idx - 1].emi.clone();
                let diff = self.periods[idx].emi.minus(&self.periods[idx - 1].emi);
                let uncountable = self.get_uncountable_periods(start, &original);
                return EmiAdj {
                    original_emi: original,
                    emi_difference: diff,
                    num_related,
                    uncountable,
                    scale: self.scale,
                    mode: self.mode,
                };
            }
            idx -= 1;
        }
        EmiAdj {
            original_emi: self.periods[start].emi.clone(),
            emi_difference: self.zero(),
            num_related,
            uncountable: 0,
            scale: self.scale,
            mode: self.mode,
        }
    }

    /// `getUncountablePeriods` — related periods whose total paid amount exceeds `original_emi`.
    fn get_uncountable_periods(&self, start: usize, original_emi: &Money) -> usize {
        (start..self.periods.len())
            .filter(|&i| is_less_than(original_emi, &self.total_paid_amount(i)))
            .count()
    }

    /// `checkAndAdjustEmiIfNeeded` — iteratively (≤3×) nudge the related periods' EMI toward balancing the last period,
    /// keeping the change only while it strictly reduces the EMI difference.
    fn check_and_adjust_emi_if_needed(&mut self, start: usize) {
        let mut counter = 1;
        while counter <= 3 {
            let adj = self.get_emi_adjustment(start);
            if !adj.should_be_adjusted() {
                break;
            }
            let adjusted_emi = adj.adjusted_emi();
            if is_equal_to(&adjusted_emi, &adj.original_emi) {
                break;
            }
            let first_from = self.periods[start].from;
            let first_due = self.periods[start].due;
            let mut copy = self.clone();
            for i in 0..copy.periods.len() {
                if copy.periods[i].from >= first_from
                    && copy.periods[i].due >= first_due
                    && !is_less_than(&adjusted_emi, &copy.total_paid_amount(i))
                {
                    copy.periods[i].emi = adjusted_emi.clone();
                    copy.periods[i].original_emi = adjusted_emi.clone();
                }
            }
            copy.calculate_outstanding_balance();
            copy.calculate_last_unpaid_emi();
            if !copy.get_emi_adjustment(start).has_less_emi_difference(&adj) {
                break;
            }
            for i in 0..self.periods.len() {
                if self.periods[i].from >= first_from && self.periods[i].due >= first_due {
                    self.periods[i].emi = copy.periods[i].emi.clone();
                    self.periods[i].original_emi = copy.periods[i].emi.clone();
                }
            }
            self.calculate_outstanding_balance();
            counter += 1;
        }
    }

    /// `changeInterestRate(date, newRatePct)` — the rate becomes `newRatePct` from `date − 1 day` onward; split the
    /// affected period at the effective date and recompute rate factors + the annuity EMI over the related periods,
    /// then run the EMI cent-adjust loop (declining balance, `onlyOnActualModelShouldApply`).
    pub fn change_interest_rate(&mut self, date: Date, new_rate_pct: &BigDecimal) {
        let effective = date - time::Duration::days(1);
        let new_rate = self.mc.div(new_rate_pct, &BigDecimal::from(100));
        self.interest_rates.push((effective, new_rate));
        let zero = self.zero();
        if let Some(rp_idx) = self.change_outstanding_and_split(effective, &zero, &zero, &zero) {
            let start = self.related_start(self.effective_due_date(rp_idx, effective));
            self.calculate_rate_factor_for_periods();
            self.calculate_outstanding_balance();
            self.calculate_emi_on_actual_model(start);
            self.calculate_outstanding_balance();
            self.calculate_last_unpaid_emi();
            self.check_and_adjust_emi_if_needed(start);
        }
    }

    /// `changeDueDate(targetDueDate, newDueDate, fixedLength)` — move the period containing `target_due` to end at
    /// `new_due`, shift every later period forward (each due = previous due + repayEvery months), with the LAST period's
    /// due overridden to `repaymentStart + (fixedLength + variationDays) months` when `fixed_length` is set (the
    /// `calculateDateForFixedLength` quirk, where `variationDays = (newDue − targetDue) in days` is added as months).
    /// Then recompute rate factors / outstanding / tail EMI (the per-period EMI is NOT re-annuitized).
    pub fn change_due_date(&mut self, target_due: Date, new_due: Date, fixed_length: Option<i64>) {
        let Some(target_idx) = self.find_period_containing(target_due) else {
            return;
        };
        let repay_every = match self.day_count {
            DayCount::Days360Days30 { repay_every } => repay_every,
            _ => 1,
        };
        self.periods[target_idx].due = new_due;
        if let Some(last) = self.periods[target_idx].interest_periods.last_mut() {
            last.due = new_due;
        }
        let last_idx = self.periods.len() - 1;
        let start = self.periods[0].from;
        let variation_days = (new_due - target_due).whole_days();
        let mut cur_new_due = new_due;
        for i in (target_idx + 1)..self.periods.len() {
            self.periods[i].from = cur_new_due;
            let mut next = self.generate_next_repayment_date(cur_new_due, repay_every);
            if i == last_idx
                && let Some(fl) = fixed_length
            {
                next = add_months(start, fl + variation_days);
            }
            self.periods[i].due = next;
            if let Some(first) = self.periods[i].interest_periods.first_mut() {
                first.from = cur_new_due;
            }
            if let Some(lastip) = self.periods[i].interest_periods.last_mut() {
                lastip.due = next;
            }
            cur_new_due = next;
        }
        self.calculate_rate_factor_for_periods();
        self.calculate_outstanding_balance();
        self.calculate_last_unpaid_emi();
    }

    /// `addRepaymentPeriods(submittedOnDate, count)` — append `count` new repayment periods (each `repayEvery` months
    /// after the schedule, generated from the seed = first period's from date), then re-amortize from the period after
    /// the one containing `submittedOnDate − 1 day`.
    pub fn add_repayment_periods(&mut self, submitted_on: Date, count: usize) {
        let Some(start_date) = self.periods.first().map(|p| p.from) else {
            return;
        };
        let repay_every = match self.day_count {
            DayCount::Days360Days30 { repay_every } => repay_every,
            _ => 1,
        };
        let existing = self.periods.len();
        let (total_disbursed, total_capitalized) = match self.periods.last() {
            Some(last) => (
                last.total_disbursed_amount.clone(),
                last.total_capitalized_income_amount.clone(),
            ),
            None => (self.zero(), self.zero()),
        };
        let zero = self.zero();
        for i in existing..(existing + count) {
            let from = add_months(start_date, i as i64 * repay_every);
            let due = add_months(start_date, (i as i64 + 1) * repay_every);
            self.periods.push(RepaymentPeriod {
                from,
                due,
                interest_periods: vec![InterestPeriod {
                    from,
                    due,
                    rate_factor: BigDecimal::from(0),
                    rate_factor_till_period_due_date: BigDecimal::from(0),
                    credited_principal: zero.clone(),
                    credited_interest: zero.clone(),
                    disbursement_amount: zero.clone(),
                    balance_correction_amount: zero.clone(),
                    capitalized_income_principal: zero.clone(),
                    outstanding_loan_balance: zero.clone(),
                    is_paused: false,
                }],
                emi: zero.clone(),
                original_emi: zero.clone(),
                paid_principal: zero.clone(),
                paid_interest: zero.clone(),
                future_unrecognized_interest: zero.clone(),
                total_disbursed_amount: total_disbursed.clone(),
                total_capitalized_income_amount: total_capitalized.clone(),
                interest_payment_grace: false,
                is_interest_moved_upward: false,
                is_interest_moved_downward: false,
                re_aged: false,
                re_aged_early_repayment_holder: false,
                credited_principal_moved_due_reage: zero.clone(),
                credited_interest_moved_due_reage: zero.clone(),
                fixed_interest: zero.clone(),
            });
        }
        let effective = submitted_on - time::Duration::days(1);
        if let Some(rp_idx) = self.change_outstanding_and_split(effective, &zero, &zero, &zero) {
            let start = self.related_start(self.effective_due_date(rp_idx, effective));
            self.calculate_rate_factor_for_periods();
            if self.interest_method == InterestMethod::Flat {
                self.calculate_emi_on_actual_model_flat(start);
            } else {
                self.calculate_outstanding_balance();
                self.calculate_emi_on_actual_model(start);
                self.calculate_outstanding_balance();
                self.calculate_last_unpaid_emi();
                self.check_and_adjust_emi_if_needed(start);
            }
        }
    }

    /// `applyInterestPause(fromDate, endDate)` — pause interest accrual over `[fromDate, endDate)`: split the affected
    /// repayment period(s) at the pause boundaries and mark the inner interest period paused (its rate factor is still
    /// computed but its calculated due interest is zero), then recompute rate factors / outstanding / tail EMI (the EMI
    /// annuity is NOT recomputed — only the last-unpaid-period diff adjustment).
    pub fn apply_interest_pause(&mut self, from: Date, end: Date) {
        if self
            .update_interest_periods_for_interest_pause(from, end)
            .is_some()
        {
            self.calculate_rate_factor_for_periods();
            self.calculate_outstanding_balance();
            self.calculate_last_unpaid_emi();
        }
    }

    /// `updateInterestPeriodsForInterestPause` — insert pause boundaries into every repayment period overlapping
    /// `[fromDate, endDate)` and return the first affected period.
    fn update_interest_periods_for_interest_pause(
        &mut self,
        from: Date,
        end: Date,
    ) -> Option<usize> {
        let affected: Vec<usize> = (0..self.periods.len())
            .filter(|&i| self.periods[i].from < end && self.periods[i].due >= from)
            .collect();
        for &i in &affected {
            self.insert_interest_pause_periods(i, from, end);
        }
        affected.first().copied()
    }

    /// `insertInterestPausePeriods` — clamp the pause window to the period and split at `pauseStart−1` / `pauseEnd`.
    fn insert_interest_pause_periods(&mut self, rp_idx: usize, pause_start: Date, pause_end: Date) {
        let effective_start = pause_start - time::Duration::days(1);
        let rp_from = self.periods[rp_idx].from;
        let rp_due = self.periods[rp_idx].due;
        let final_start = if effective_start < rp_from {
            rp_from
        } else {
            effective_start
        };
        let final_end = if pause_end > rp_due {
            rp_due
        } else {
            pause_end
        };
        let zero = self.zero();
        if !self.periods[rp_idx]
            .interest_periods
            .iter()
            .any(|ip| ip.from == final_start)
        {
            self.insert_interest_period(rp_idx, final_start, &zero, &zero, &zero);
        }
        if !self.periods[rp_idx]
            .interest_periods
            .iter()
            .any(|ip| ip.due == final_end)
        {
            self.insert_interest_period(rp_idx, final_end, &zero, &zero, &zero);
        }
        for ip in &mut self.periods[rp_idx].interest_periods {
            if ip.from >= final_start && ip.due <= final_end {
                ip.is_paused = true;
            }
        }
    }

    /// `getDueAmounts(periodFromDate, periodDueDate, targetDate)` for the horizontal (default) processing type.
    #[must_use]
    pub fn get_due_amounts(&self, from: Date, due: Date, target: Date) -> Option<PeriodDueDetails> {
        let mut recalc = self.recalculate_till_date(target);
        let rp_idx = recalc.find_repayment_period(from, due)?;
        let not_fully_repaid = (0..recalc.periods.len())
            .filter(|&i| !recalc.is_fully_paid(i))
            .count();
        recalc.adjust_emi_if_required(rp_idx, target, not_fully_repaid);
        Some(PeriodDueDetails {
            emi: recalc.period_emi(rp_idx),
            due_principal: recalc.due_principal(rp_idx),
            due_interest: recalc.due_interest(rp_idx),
        })
    }

    /// `getPeriodInterestTillDate(from, due, targetDate, includeCreditedInterest, fixedInterestTillDate)` — the
    /// calculated due interest of the period accrued only up to `target` (used for daily-interest accrual). The
    /// `fixedInterestTillDate` adjustment is a no-op while `fixedInterest` is zero (the only case the engine supports).
    #[must_use]
    pub fn get_period_interest_till_date(
        &self,
        from: Date,
        due: Date,
        target: Date,
        include_credited_interest: bool,
    ) -> Option<Money> {
        let recalc = self.recalculate_till_date(target);
        let rp_idx = recalc.find_repayment_period(from, due)?;
        let calc = recalc.calculated_due_interest(rp_idx);
        if include_credited_interest {
            Some(calc)
        } else {
            Some(calc.minus(&recalc.credited_interest(rp_idx)))
        }
    }

    /// `calculateRateFactorForScheduleTillDateInclusive` — clip the interest period containing `target` to end at
    /// `target`, recompute rate factors, then zero the rate factors of any interest period entirely after `target`.
    fn calculate_rate_factor_for_schedule_till_date_inclusive(&mut self, target: Date) {
        if let Some(rp_idx) = self.find_period_containing(target)
            && let Some(k) = self.find_interest_period_in(rp_idx, target)
        {
            self.periods[rp_idx].interest_periods[k].due = target;
        }
        self.calculate_rate_factor_for_periods();
        for rp in &mut self.periods {
            for ip in &mut rp.interest_periods {
                if target < ip.due {
                    ip.rate_factor = BigDecimal::from(0);
                    ip.rate_factor_till_period_due_date = BigDecimal::from(0);
                }
            }
        }
    }

    /// `getOutstandingAmountsTillDate(targetDate)` — total outstanding principal and interest accrued only up to
    /// `target` (interest beyond `target` clipped to zero). Returns `(outstanding_principal, outstanding_interest)`.
    #[must_use]
    pub fn get_outstanding_amounts_till_date(&self, target: Date) -> (Money, Money) {
        let mut copy = self.clone();
        copy.calculate_rate_factor_for_schedule_till_date_inclusive(target);
        copy.calculate_outstanding_balance();
        copy.calculate_last_unpaid_emi();
        let principal = neg_to_zero(
            &copy
                .total_due_principal()
                .minus(&copy.total_paid_principal()),
            self.scale,
            self.mode,
        );
        let interest = neg_to_zero(
            &copy.total_due_interest().minus(&copy.total_paid_interest()),
            self.scale,
            self.mode,
        );
        (principal, interest)
    }

    /// `RepaymentPeriod.getOutstandingInterest()` = negativeToZero(dueInterest − paidInterest).
    fn outstanding_interest(&self, i: usize) -> Money {
        neg_to_zero(
            &self.due_interest(i).minus(&self.periods[i].paid_interest),
            self.scale,
            self.mode,
        )
    }

    /// `accelerateRepaymentDueDateTo` — move the period (and its last interest period) due date to `date`, recompute.
    fn accelerate_repayment_due_date_to(&mut self, rp_idx: usize, date: Date) {
        self.periods[rp_idx].due = date;
        if let Some(last) = self.periods[rp_idx].interest_periods.last_mut() {
            last.due = date;
        }
        self.calculate_rate_factor_for_repayment_period(rp_idx);
    }

    /// `accelerateMaturityDateTo` — flag the period containing `date` as the re-aged early-repayment holder, accelerate
    /// its due date to `date`, and drop all periods after it.
    fn accelerate_maturity_date_to(&mut self, date: Date) {
        let Some(rp_idx) = self.find_period_containing(date) else {
            return;
        };
        self.periods[rp_idx].re_aged_early_repayment_holder = true;
        if self.periods[rp_idx].due != date {
            self.accelerate_repayment_due_date_to(rp_idx, date);
        }
        if rp_idx != self.periods.len() - 1 {
            self.periods.retain(|p| p.due <= date);
        }
    }

    /// `calculatePaidBalancesAfterDate` — sum of paid principal / interest in periods due on or after `date`.
    fn calculate_paid_balances_after_date(&self, date: Date) -> (Money, Money) {
        let mut principal = self.zero();
        let mut interest = self.zero();
        for p in &self.periods {
            if p.due >= date {
                principal = principal.plus(&p.paid_principal);
                interest = interest.plus(&p.paid_interest);
            }
        }
        (principal, interest)
    }

    /// `createRepaymentPeriodForEarlyRepaidAmountsDuringReAgeing` — fold the already-paid balances into the last period.
    fn create_repayment_period_for_early_repaid(
        &mut self,
        total_paid_principal: &Money,
        total_paid_interest: &Money,
        is_re_aged: bool,
    ) {
        let last = self.periods.len() - 1;
        let pi = total_paid_interest.minus(&self.periods[last].paid_interest);
        let pp = total_paid_principal.minus(&self.periods[last].paid_principal);
        self.periods[last].paid_interest = self.periods[last].paid_interest.plus(&pi);
        self.periods[last].paid_principal = self.periods[last].paid_principal.plus(&pp);
        self.periods[last].emi = self.total_paid_amount(last);
        if is_re_aged {
            self.periods[last].re_aged = true;
        }
        self.periods[last].re_aged_early_repayment_holder = true;
        self.calculate_rate_factor_for_repayment_period(last);
    }

    fn is_collapsable_stub(&self, i: usize) -> bool {
        self.periods[i].re_aged_early_repayment_holder
            && self.periods[i].emi.is_zero()
            && self.due_principal(i).is_zero()
            && self.due_interest(i).is_zero()
    }

    /// `collapseIntermediateStubPeriods` — merge adjacent collapsable re-age stub periods.
    fn collapse_intermediate_stub_periods(&mut self) {
        if self.periods.len() <= 1 {
            return;
        }
        let mut i = 0;
        while i < self.periods.len() {
            if self.is_collapsable_stub(i) {
                while i + 1 < self.periods.len() && self.is_collapsable_stub(i + 1) {
                    let removed_due = self.periods.remove(i + 1).due;
                    self.periods[i].due = removed_due;
                    if let Some(last) = self.periods[i].interest_periods.last_mut() {
                        last.due = removed_due;
                    }
                    self.calculate_rate_factor_for_repayment_period(i);
                }
            }
            i += 1;
        }
    }

    /// `updateModelForReageEqualAmortization` — append `n` new re-aged repayment periods starting at `start_date`,
    /// stepping by `frequency_months`.
    fn update_model_for_reage(&mut self, start_date: Date, n: usize, frequency_months: i64) {
        let mut to_date = start_date;
        for _ in 0..n {
            let last = self.periods.len() - 1;
            let from = self.periods[last].due;
            let (total_disbursed, total_capitalized) = (
                self.periods[last].total_disbursed_amount.clone(),
                self.periods[last].total_capitalized_income_amount.clone(),
            );
            let zero = self.zero();
            self.periods.push(RepaymentPeriod {
                from,
                due: to_date,
                interest_periods: vec![InterestPeriod {
                    from,
                    due: to_date,
                    rate_factor: BigDecimal::from(0),
                    rate_factor_till_period_due_date: BigDecimal::from(0),
                    credited_principal: zero.clone(),
                    credited_interest: zero.clone(),
                    disbursement_amount: zero.clone(),
                    balance_correction_amount: zero.clone(),
                    capitalized_income_principal: zero.clone(),
                    outstanding_loan_balance: zero.clone(),
                    is_paused: false,
                }],
                emi: zero.clone(),
                original_emi: zero.clone(),
                paid_principal: zero.clone(),
                paid_interest: zero.clone(),
                future_unrecognized_interest: zero.clone(),
                total_disbursed_amount: total_disbursed,
                total_capitalized_income_amount: total_capitalized,
                interest_payment_grace: false,
                is_interest_moved_upward: false,
                is_interest_moved_downward: false,
                re_aged: true,
                re_aged_early_repayment_holder: false,
                credited_principal_moved_due_reage: zero.clone(),
                credited_interest_moved_due_reage: zero.clone(),
                fixed_interest: zero,
            });
            to_date = add_months(to_date, frequency_months);
        }
    }

    /// `updateEMIForReAgeEqualAmortization` — distribute the re-aged principal + interest equally over the last `n`
    /// (re-aged) periods, recording each period's `fixed_interest` and EMI.
    fn update_emi_for_reage(&mut self, n: usize, principal: &Money, interest: &Money) {
        use crate::equal_amortization::{
            calculate_adjusted_equal_amortization_values, calculate_equal_amortization_values,
        };
        let installments = i64::try_from(n).unwrap_or(0);
        let interest_eav = calculate_equal_amortization_values(interest, installments);
        let total = principal.plus(interest);
        let principal_eav = calculate_adjusted_equal_amortization_values(
            principal,
            &total,
            &interest_eav.value,
            installments,
        );
        // Divergence note (deny-panic totalization): Java's `EqualAmortizationValues.add` throws a
        // `RuntimeException` on an installment-count mismatch; here the whole EMI update would be silently skipped
        // instead. Both value sets are built from the SAME `installments` count above, so the mismatch is
        // unreachable by construction — Java's throw can never fire at this call site either.
        let emi_eav = match principal_eav.try_add(&interest_eav) {
            Ok(v) => v,
            Err(_) => return,
        };
        let base = self.periods.len() - n;
        for j in 0..n {
            let idx = base + j;
            let ji = i64::try_from(j).unwrap_or(0);
            self.periods[idx].fixed_interest = interest_eav.calculate_value(ji);
            let emi = emi_eav.calculate_value(ji);
            self.periods[idx].emi = emi.clone();
            self.periods[idx].original_emi = emi;
        }
    }

    /// `reAgeEqualAmortization` — re-age the loan: accelerate the maturity to `transaction_date`, close the open
    /// periods (keeping paid amounts), then append `number_of_installments` new equal-amortization periods from
    /// `re_age_start_date` carrying the re-aged outstanding principal + interest.
    pub fn re_age_equal_amortization(
        &mut self,
        transaction_date: Date,
        re_age_start_date: Date,
        number_of_installments: usize,
        frequency_months: i64,
        full_interest: bool,
    ) {
        let original_maturity = self.periods.last().map(|p| p.due);
        let amount_target = if full_interest {
            original_maturity
        } else {
            Some(transaction_date)
        };
        let (reage_principal, reage_interest) = match amount_target {
            Some(t) => self.get_outstanding_amounts_till_date(t),
            None => (self.zero(), self.zero()),
        };
        let paid = self.calculate_paid_balances_after_date(transaction_date);
        let mut future_credited = self.zero();
        for i in 0..self.periods.len() {
            if self.periods[i].from >= transaction_date && self.periods[i].due > transaction_date {
                future_credited = future_credited.plus(&self.credited_principal(i));
            }
        }

        self.accelerate_maturity_date_to(transaction_date);
        let zero = self.zero();
        self.add_credit(transaction_date, &future_credited, &zero);

        // Close all open repayment periods while keeping paid amounts.
        for i in 0..self.periods.len() {
            let oi = self.outstanding_interest(i);
            let ci = self.credited_interest(i);
            let neg = self.zero().minus(&money_min(&oi, &ci));
            if let Some(last) = self.periods[i].interest_periods.last_mut() {
                last.credited_interest = last.credited_interest.plus(&neg);
            }
            self.periods[i].emi = self.total_paid_amount(i);
            self.periods[i].credited_principal_moved_due_reage = self.credited_principal(i);
            self.periods[i].credited_interest_moved_due_reage = self.credited_interest(i);
            self.periods[i].is_interest_moved_downward = true;
        }

        if original_maturity.is_some_and(|om| om >= transaction_date) {
            self.create_repayment_period_for_early_repaid(&paid.0, &paid.1, true);
        }

        self.collapse_intermediate_stub_periods();
        self.update_model_for_reage(re_age_start_date, number_of_installments, frequency_months);
        self.update_emi_for_reage(number_of_installments, &reage_principal, &reage_interest);
        self.calculate_outstanding_balance();

        let zero_from = match original_maturity {
            Some(om) if om < transaction_date => om,
            _ => transaction_date,
        };
        self.interest_rates.push((zero_from, BigDecimal::from(0)));
        self.calculate_last_unpaid_emi();
    }
}

/// Whole months between `a` and `b` (`ChronoUnit.MONTHS.between` semantics): `(b.year−a.year)*12 + (b.month−a.month)`,
/// minus 1 when `b`'s day-of-month is before `a`'s.
fn months_between(a: Date, b: Date) -> i64 {
    let mut months = (i64::from(b.year()) - i64::from(a.year())) * 12
        + (i64::from(u8::from(b.month())) - i64::from(u8::from(a.month())));
    if b.day() < a.day() {
        months -= 1;
    }
    months
}

/// `getRepaymentPeriodDate(MONTHS, n, date)` — add `n` months, clamping the day to the target month's length. This is
/// the simple frequency add used by re-aging (NOT the off-cadence `generateNextRepaymentDate`).
fn add_months(date: Date, n: i64) -> Date {
    let total = i64::from(date.year()) * 12 + i64::from(u8::from(date.month())) - 1 + n;
    let year = i32::try_from(total.div_euclid(12)).unwrap_or(date.year());
    let month_idx = u8::try_from(total.rem_euclid(12) + 1).unwrap_or(1);
    let month = time::Month::try_from(month_idx).unwrap_or(time::Month::January);
    let mut day = date.day();
    while day > 1 {
        if let Ok(d) = Date::from_calendar_date(year, month, day) {
            return d;
        }
        day -= 1;
    }
    Date::from_calendar_date(year, month, 1).unwrap_or(date)
}

/// Build a fresh declining-balance model for DAYS_360 / DAYS_30 and apply the initial disbursement — equivalent to
/// `generatePeriodInterestScheduleModel` + `addDisbursement`.
#[must_use]
pub fn new_declining_balance(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    repay_every: i64,
    scale: i64,
    mode: RoundingMode,
) -> Model {
    new_declining_balance_with_day_count(
        period_dates,
        disbursed,
        nominal_rate_pct,
        DayCount::Days360Days30 { repay_every },
        scale,
        mode,
    )
}

/// As [`new_declining_balance`] but with an explicit [`DayCount`] (e.g. ACTUAL/ACTUAL).
#[must_use]
pub fn new_declining_balance_with_day_count(
    period_dates: &[(Date, Date)],
    disbursed: &BigDecimal,
    nominal_rate_pct: &BigDecimal,
    day_count: DayCount,
    scale: i64,
    mode: RoundingMode,
) -> Model {
    let mc = Mc::new(12, mode);
    let rate = mc.div(nominal_rate_pct, &BigDecimal::from(100));
    let zero = Money::zero(scale, mode);
    let periods = period_dates
        .iter()
        .map(|(from, due)| RepaymentPeriod {
            from: *from,
            due: *due,
            interest_periods: vec![InterestPeriod {
                from: *from,
                due: *due,
                rate_factor: BigDecimal::from(0),
                rate_factor_till_period_due_date: BigDecimal::from(0),
                credited_principal: zero.clone(),
                credited_interest: zero.clone(),
                disbursement_amount: zero.clone(),
                balance_correction_amount: zero.clone(),
                capitalized_income_principal: zero.clone(),
                outstanding_loan_balance: zero.clone(),
                is_paused: false,
            }],
            emi: zero.clone(),
            original_emi: zero.clone(),
            paid_principal: zero.clone(),
            paid_interest: zero.clone(),
            future_unrecognized_interest: zero.clone(),
            total_disbursed_amount: zero.clone(),
            total_capitalized_income_amount: zero.clone(),
            interest_payment_grace: false,
            is_interest_moved_upward: false,
            is_interest_moved_downward: false,
            re_aged: false,
            re_aged_early_repayment_holder: false,
            credited_principal_moved_due_reage: zero.clone(),
            credited_interest_moved_due_reage: zero.clone(),
            fixed_interest: zero.clone(),
        })
        .collect();
    let mut model = Model {
        periods,
        mc,
        scale,
        mode,
        rate,
        interest_rates: Vec::new(),
        day_count,
        interest_method: InterestMethod::DecliningBalance,
        emi_recalculation_enabled: true,
        allow_partial_period_interest: true,
        grace_on_interest: 0,
        interest_pause_for_emi: false,
        allow_full_term_for_tranche: false,
        number_of_repayments: 0,
    };
    if let (Some((from, _)), _) = (period_dates.first(), ()) {
        model.add_disbursement(*from, &Money::of(scale, mode, disbursed));
    }
    model
}

/// Build an EMPTY model (no disbursement applied) with the given interest method — for multi-disbursement scenarios
/// (especially FLAT) where the caller applies disbursements explicitly via [`Model::add_disbursement`].
#[must_use]
pub fn new_empty(
    period_dates: &[(Date, Date)],
    nominal_rate_pct: &BigDecimal,
    day_count: DayCount,
    interest_method: InterestMethod,
    scale: i64,
    mode: RoundingMode,
) -> Model {
    let mc = Mc::new(12, mode);
    let rate = mc.div(nominal_rate_pct, &BigDecimal::from(100));
    let zero = Money::zero(scale, mode);
    let periods = period_dates
        .iter()
        .map(|(from, due)| RepaymentPeriod {
            from: *from,
            due: *due,
            interest_periods: vec![InterestPeriod {
                from: *from,
                due: *due,
                rate_factor: BigDecimal::from(0),
                rate_factor_till_period_due_date: BigDecimal::from(0),
                credited_principal: zero.clone(),
                credited_interest: zero.clone(),
                disbursement_amount: zero.clone(),
                balance_correction_amount: zero.clone(),
                capitalized_income_principal: zero.clone(),
                outstanding_loan_balance: zero.clone(),
                is_paused: false,
            }],
            emi: zero.clone(),
            original_emi: zero.clone(),
            paid_principal: zero.clone(),
            paid_interest: zero.clone(),
            future_unrecognized_interest: zero.clone(),
            total_disbursed_amount: zero.clone(),
            total_capitalized_income_amount: zero.clone(),
            interest_payment_grace: false,
            is_interest_moved_upward: false,
            is_interest_moved_downward: false,
            re_aged: false,
            re_aged_early_repayment_holder: false,
            credited_principal_moved_due_reage: zero.clone(),
            credited_interest_moved_due_reage: zero.clone(),
            fixed_interest: zero.clone(),
        })
        .collect();
    Model {
        periods,
        mc,
        scale,
        mode,
        rate,
        interest_rates: Vec::new(),
        day_count,
        interest_method,
        emi_recalculation_enabled: true,
        allow_partial_period_interest: true,
        grace_on_interest: 0,
        interest_pause_for_emi: false,
        allow_full_term_for_tranche: false,
        number_of_repayments: 0,
    }
}

/// Build an EMPTY SAME_AS_REPAYMENT model with the partial-period flag — for the sameAsRepayment multi-disbursement
/// scenarios. `months_in_year` is 12 (monthly) / 52 (weekly); `allow_partial` is `isAllowPartialPeriodInterestCalculation`.
#[allow(clippy::too_many_arguments)]
pub fn new_empty_same_as_repayment(
    period_dates: &[(Date, Date)],
    nominal_rate_pct: &BigDecimal,
    repay_every: i64,
    months_in_year: i64,
    allow_partial: bool,
    interest_method: InterestMethod,
    scale: i64,
    mode: RoundingMode,
) -> Model {
    let mut m = new_empty(
        period_dates,
        nominal_rate_pct,
        DayCount::SameAsRepayment {
            repay_every,
            months_in_year,
        },
        interest_method,
        scale,
        mode,
    );
    m.allow_partial_period_interest = allow_partial;
    m
}
