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

/// Day count — currently DAYS_360 / DAYS_30 with a repay-every multiplier (the `test_payoff_*` family).
#[derive(Clone, Copy, Debug)]
pub enum DayCount {
    Days360Days30 { repay_every: i64 },
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
    pub future_unrecognized_interest: Money,
    pub total_disbursed_amount: Money,
    pub total_capitalized_income_amount: Money,
    pub interest_payment_grace: bool,
    pub is_interest_moved_upward: bool,
    pub is_interest_moved_downward: bool,
    pub re_aged: bool,
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
#[derive(Clone, Debug)]
pub struct Model {
    pub periods: Vec<RepaymentPeriod>,
    pub mc: Mc,
    pub scale: i64,
    pub mode: RoundingMode,
    /// Nominal interest rate as a fraction (annual % / 100).
    pub rate: BigDecimal,
    pub day_count: DayCount,
    pub interest_method: InterestMethod,
    pub emi_recalculation_enabled: bool,
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

impl Model {
    fn money(&self, v: &BigDecimal) -> Money {
        Money::of(self.scale, self.mode, v)
    }
    fn zero(&self) -> Money {
        Money::zero(self.scale, self.mode)
    }

    // --- rate factors ---

    /// `calculateRateFactorPerPeriod` — the per-interest-period rate factor (proportional to the interest-period length
    /// within the repayment period): DAYS_360/DAYS_30 → `rate × 30 × repayEvery / 360 × ipLen / rpLen`.
    fn rate_factor_ip(
        &self,
        ip_from: Date,
        ip_due: Date,
        rp_from: Date,
        rp_due: Date,
    ) -> BigDecimal {
        match self.day_count {
            DayCount::Days360Days30 { repay_every } => rate_factor_by_repayment_period(
                &self.mc,
                &self.rate,
                &BigDecimal::from(30),
                &BigDecimal::from(repay_every),
                &BigDecimal::from(360),
                &BigDecimal::from((ip_due - ip_from).whole_days()),
                &BigDecimal::from((rp_due - rp_from).whole_days()),
            ),
        }
    }

    /// `calculateRateFactorPerPeriodForInterest(ip.from, rp.due)` — the rate factor used for interest accrual
    /// (`rateFactorTillPeriodDueDate`): DAYS_360/DAYS_30 → `rate × 30 × periodRatio / 360 × (rpDue − ipFrom) / rpLen`
    /// with `periodRatio = repayEvery` for a regular monthly schedule.
    fn rate_factor_till(&self, ip_from: Date, rp_from: Date, rp_due: Date) -> BigDecimal {
        match self.day_count {
            DayCount::Days360Days30 { repay_every } => rate_factor_by_repayment_period(
                &self.mc,
                &self.rate,
                &BigDecimal::from(30),
                &BigDecimal::from(repay_every),
                &BigDecimal::from(360),
                &BigDecimal::from((rp_due - ip_from).whole_days()),
                &BigDecimal::from((rp_due - rp_from).whole_days()),
            ),
        }
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
        calculated_due_interest_declining(
            &self.mc,
            ip.outstanding_loan_balance.amount(),
            &ip.rate_factor_till_period_due_date,
            length,
            length_till,
            ip.credited_interest.amount(),
        )
    }

    /// `RepaymentPeriod.getEmiPlusCreditedAmountsPlusFutureUnrecognizedInterest()`.
    fn emi_plus_credited_plus_future(&self, i: usize) -> Money {
        let rp = &self.periods[i];
        rp.emi
            .plus(&self.total_credited_amount(i))
            .plus(&rp.future_unrecognized_interest)
    }

    /// `RepaymentPeriod.getTotalCreditedAmount()` (no re-age moves in this slice).
    fn total_credited_amount(&self, i: usize) -> Money {
        self.credited_principal(i).plus(&self.credited_interest(i))
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

    /// `RepaymentPeriod.calculateCalculatedDueInterest()`.
    fn calculated_due_interest(&self, i: usize) -> Money {
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
            acc = acc.plus(&self.unrecognized_interest(i - 1));
        }
        neg_to_zero(&acc, self.scale, self.mode)
    }

    /// `RepaymentPeriod.getCalculatedDuePrincipal()`.
    fn calculated_due_principal(&self, i: usize) -> Money {
        neg_to_zero(
            &self
                .emi_plus_credited_plus_future(i)
                .minus(&self.calculated_due_interest(i)),
            self.scale,
            self.mode,
        )
    }

    /// `RepaymentPeriod.getDueInterest()`.
    fn due_interest(&self, i: usize) -> Money {
        let rp = &self.periods[i];
        if rp.interest_payment_grace {
            return rp.paid_interest.clone();
        }
        let calc_interest = self.calculated_due_interest(i);
        let primary = if rp
            .paid_principal
            .is_greater_than(&self.calculated_due_principal(i))
        {
            rp.paid_interest.clone()
        } else {
            money_min(&calc_interest, &self.emi_plus_credited_plus_future(i))
        };
        money_max(&primary, &rp.paid_interest)
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

    /// `RepaymentPeriod.getUnrecognizedInterest()`.
    fn unrecognized_interest(&self, i: usize) -> Money {
        neg_to_zero(
            &self.calculated_due_interest(i).minus(&self.due_interest(i)),
            self.scale,
            self.mode,
        )
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
        // rateFactorN = stripTrailingZeros( Π (1 + rateFactor) ) over [start..]
        let rate_factor_n = (start..n)
            .fold(BigDecimal::from(1), |acc, i| {
                self.mc.mul(&acc, &self.rate_factor_plus_1(i))
            })
            .normalized();
        // fnResult = stripTrailingZeros( fold fnValue over [start+1..] )
        let fn_result = (start + 1..n)
            .fold(BigDecimal::from(1), |f, i| {
                fn_value(&self.mc, &f, &self.rate_factor_plus_1(i))
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
    pub fn add_disbursement(&mut self, date: Date, amount: &Money) {
        for rp in &mut self.periods {
            if date <= rp.from {
                rp.total_disbursed_amount = rp.total_disbursed_amount.plus(amount);
            }
        }
        if let Some(rp_idx) =
            self.change_outstanding_and_split(date, amount, &self.zero(), &self.zero())
        {
            // calculateEMIValueAndRateFactorsForDecliningBalanceInterestMethod on the related periods.
            let related_start = self.related_start(self.effective_due_date(rp_idx, date));
            self.calculate_rate_factor_for_periods();
            self.calculate_outstanding_balance();
            self.calculate_emi_on_actual_model(related_start);
            self.calculate_outstanding_balance();
            self.calculate_last_unpaid_emi();
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
            fixed_interest: zero.clone(),
        })
        .collect();
    let mut model = Model {
        periods,
        mc,
        scale,
        mode,
        rate,
        day_count: DayCount::Days360Days30 { repay_every },
        interest_method: InterestMethod::DecliningBalance,
        emi_recalculation_enabled: true,
    };
    if let (Some((from, _)), _) = (period_dates.first(), ()) {
        model.add_disbursement(*from, &Money::of(scale, mode, disbursed));
    }
    model
}
