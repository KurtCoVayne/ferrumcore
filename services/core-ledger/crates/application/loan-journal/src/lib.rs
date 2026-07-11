//! `fc-loan-journal` — the append-only per-loan **operation log** and its deterministic **replay** into the EMI
//! interest-schedule model (ADR-0026: event-log recompute, not model-blob persistence).
//!
//! The kernel (`fc-loan-emi`) is value-semantic and serde-free by design (Part-4 invariant + thread-safety). This
//! application-layer crate is the persistence *projection* over it: a serde-serializable [`LoanJournal`] records the
//! ordered sequence of engine operations a loan underwent, and [`LoanJournal::replay`] reconstructs the
//! [`fc_loan_emi::recalc::Model`] by re-applying them through the kernel. The log is the source of truth; the Model is
//! a derived, disposable projection. A restart reloads the log and replays; the byte-exact reconstruction is enforced
//! by the `replay == live` golden tests.
//!
//! Encoding (ADR-0005): monetary amounts are **exact-decimal strings**, dates are **ISO-8601 `YYYY-MM-DD`**, so the
//! serialized form never contains a binary float and is stable across releases (unlike Fineract's internal-graph JSON
//! blob). The currency rounding mode is fixed to banker's rounding (`HALF_EVEN`) — the only mode the system uses.
#![forbid(unsafe_code)]

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_loan_emi::recalc::{
    DayCount, Model, new_declining_balance, new_declining_balance_with_day_count,
};
use fc_money::Money;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use time::{Date, Month};

/// The single rounding mode the whole system uses (banker's rounding). Not serialized — implied.
const MODE: RoundingMode = RoundingMode::HalfEven;

/// A replay failure: a malformed date or amount string in the persisted log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// A date field was not a valid `YYYY-MM-DD`.
    BadDate(String),
    /// An amount field was not a valid decimal string.
    BadAmount(String),
}

impl core::fmt::Display for JournalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            JournalError::BadDate(s) => write!(f, "invalid date in loan journal: {s:?}"),
            JournalError::BadAmount(s) => write!(f, "invalid amount in loan journal: {s:?}"),
        }
    }
}

impl std::error::Error for JournalError {}

/// Parse an ISO-8601 `YYYY-MM-DD` date without pulling `time`'s parsing feature.
fn parse_date(s: &str) -> Result<Date, JournalError> {
    let err = || JournalError::BadDate(s.to_string());
    let mut parts = s.split('-');
    let year: i32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    let month_num: u8 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    let day: u8 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    if parts.next().is_some() {
        return Err(err());
    }
    let month = Month::try_from(month_num).map_err(|_| err())?;
    Date::from_calendar_date(year, month, day).map_err(|_| err())
}

/// Format a date as ISO-8601 `YYYY-MM-DD` (the serialized form).
#[must_use]
pub fn format_date(d: Date) -> String {
    format!("{:04}-{:02}-{:02}", d.year(), u8::from(d.month()), d.day())
}

fn parse_decimal(s: &str) -> Result<BigDecimal, JournalError> {
    BigDecimal::from_str(s).map_err(|_| JournalError::BadAmount(s.to_string()))
}

/// How the fresh interest-schedule model is generated before any operation is applied — the declining-balance
/// constructor family of the kernel, with the day-count convention that determines the rate factors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleInit {
    /// [`new_declining_balance`] — DAYS_360 / DAYS_30 with the repay-every multiplier.
    Declining {
        /// `(from, due)` period bounds as ISO dates.
        periods: Vec<(String, String)>,
        /// Disbursed principal (decimal string).
        principal: String,
        /// Nominal annual interest rate percent (decimal string).
        rate_pct: String,
        /// Repay-every multiplier in months.
        repay_every: i64,
        /// Currency minor-unit scale.
        scale: i64,
    },
    /// [`new_declining_balance_with_day_count`] with `ActualFixedYear { days_in_year }` (e.g. DAYS_365 / DAYS_364).
    DecliningActualFixedYear {
        periods: Vec<(String, String)>,
        principal: String,
        rate_pct: String,
        /// Fixed days-in-year for the rate factor.
        days_in_year: i64,
        scale: i64,
    },
}

impl ScheduleInit {
    fn build(&self) -> Result<Model, JournalError> {
        let (periods, principal, rate_pct, scale) = match self {
            ScheduleInit::Declining {
                periods,
                principal,
                rate_pct,
                scale,
                ..
            }
            | ScheduleInit::DecliningActualFixedYear {
                periods,
                principal,
                rate_pct,
                scale,
                ..
            } => (periods, principal, rate_pct, *scale),
        };
        let period_dates: Vec<(Date, Date)> = periods
            .iter()
            .map(|(from, due)| Ok((parse_date(from)?, parse_date(due)?)))
            .collect::<Result<_, JournalError>>()?;
        let disbursed = parse_decimal(principal)?;
        let rate = parse_decimal(rate_pct)?;
        Ok(match self {
            ScheduleInit::Declining { repay_every, .. } => {
                new_declining_balance(&period_dates, &disbursed, &rate, *repay_every, scale, MODE)
            }
            ScheduleInit::DecliningActualFixedYear { days_in_year, .. } => {
                new_declining_balance_with_day_count(
                    &period_dates,
                    &disbursed,
                    &rate,
                    DayCount::ActualFixedYear {
                        days_in_year: *days_in_year,
                    },
                    scale,
                    MODE,
                )
            }
        })
    }
}

/// One recorded engine operation. Each variant maps 1:1 to a public `fc_loan_emi::recalc::Model` mutator, so the log
/// is exactly the sequence of things that happened to the loan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum LoanOp {
    /// `add_disbursement(date, amount)`.
    Disburse { date: String, amount: String },
    /// `add_capitalized_income(date, amount)`.
    CapitalizedIncome { date: String, amount: String },
    /// `pay_principal(from, due, pay_date, amount)`.
    PayPrincipal {
        from: String,
        due: String,
        pay_date: String,
        amount: String,
    },
    /// `pay_interest(from, due, pay_date, amount)`.
    PayInterest {
        from: String,
        due: String,
        pay_date: String,
        amount: String,
    },
    /// `credit_principal(date, amount)`.
    CreditPrincipal { date: String, amount: String },
    /// `credit_interest(date, amount)`.
    CreditInterest { date: String, amount: String },
    /// `add_balance_correction(date, amount)`.
    BalanceCorrection { date: String, amount: String },
    /// `change_interest_rate(date, new_rate_pct)`.
    ChangeInterestRate { date: String, rate_pct: String },
    /// `change_due_date(target_due, new_due, fixed_length)`.
    ChangeDueDate {
        target_due: String,
        new_due: String,
        fixed_length: Option<i64>,
    },
    /// `add_repayment_periods(submitted_on, count)`.
    AddRepaymentPeriods { submitted_on: String, count: usize },
    /// `apply_interest_pause(from, end)`.
    InterestPause { from: String, end: String },
    /// `re_age_equal_amortization(transaction_date, re_age_start, installments, frequency_months, full_interest)`.
    ReAge {
        transaction_date: String,
        re_age_start: String,
        installments: usize,
        frequency_months: i64,
        full_interest: bool,
    },
}

/// The persisted, replayable history of one loan's interest-schedule model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoanJournal {
    /// How the fresh model is generated.
    pub init: ScheduleInit,
    /// The ordered operations applied after generation.
    pub ops: Vec<LoanOp>,
}

impl LoanJournal {
    /// Reconstruct the interest-schedule [`Model`] by generating the fresh schedule and replaying every operation
    /// through the kernel, in order. Deterministic: the kernel is a pure function of the log (no wall-clock, no
    /// interior mutability), so `replay` of a journal always yields the same Model as the live sequence that produced
    /// it — the property the `replay == live` golden tests enforce.
    pub fn replay(&self) -> Result<Model, JournalError> {
        let mut model = self.init.build()?;
        let scale = model.scale;
        let money = |s: &str| -> Result<Money, JournalError> {
            Ok(Money::of(scale, MODE, &parse_decimal(s)?))
        };
        for op in &self.ops {
            match op {
                LoanOp::Disburse { date, amount } => {
                    model.add_disbursement(parse_date(date)?, &money(amount)?);
                }
                LoanOp::CapitalizedIncome { date, amount } => {
                    model.add_capitalized_income(parse_date(date)?, &money(amount)?);
                }
                LoanOp::PayPrincipal {
                    from,
                    due,
                    pay_date,
                    amount,
                } => {
                    model.pay_principal(
                        parse_date(from)?,
                        parse_date(due)?,
                        parse_date(pay_date)?,
                        &money(amount)?,
                    );
                }
                LoanOp::PayInterest {
                    from,
                    due,
                    pay_date,
                    amount,
                } => {
                    model.pay_interest(
                        parse_date(from)?,
                        parse_date(due)?,
                        parse_date(pay_date)?,
                        &money(amount)?,
                    );
                }
                LoanOp::CreditPrincipal { date, amount } => {
                    model.credit_principal(parse_date(date)?, &money(amount)?);
                }
                LoanOp::CreditInterest { date, amount } => {
                    model.credit_interest(parse_date(date)?, &money(amount)?);
                }
                LoanOp::BalanceCorrection { date, amount } => {
                    model.add_balance_correction(parse_date(date)?, &money(amount)?);
                }
                LoanOp::ChangeInterestRate { date, rate_pct } => {
                    model.change_interest_rate(parse_date(date)?, &parse_decimal(rate_pct)?);
                }
                LoanOp::ChangeDueDate {
                    target_due,
                    new_due,
                    fixed_length,
                } => {
                    model.change_due_date(
                        parse_date(target_due)?,
                        parse_date(new_due)?,
                        *fixed_length,
                    );
                }
                LoanOp::AddRepaymentPeriods {
                    submitted_on,
                    count,
                } => {
                    model.add_repayment_periods(parse_date(submitted_on)?, *count);
                }
                LoanOp::InterestPause { from, end } => {
                    model.apply_interest_pause(parse_date(from)?, parse_date(end)?);
                }
                LoanOp::ReAge {
                    transaction_date,
                    re_age_start,
                    installments,
                    frequency_months,
                    full_interest,
                } => {
                    model.re_age_equal_amortization(
                        parse_date(transaction_date)?,
                        parse_date(re_age_start)?,
                        *installments,
                        *frequency_months,
                        *full_interest,
                    );
                }
            }
        }
        Ok(model)
    }
}
