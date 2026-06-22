//! # fc-domain-charge-tax-rates
//!
//! Charge / tax / floating-rate domain. Pure: no IO, no framework. Codes are faithful to the original (wire/DB
//! contract); algorithms reproduce the original behavior — **including the tax computation's IEEE-`double` quirk**
//! (`TaxUtils.splitTax`), which is preserved for bit-parity and flagged for post-parity review.
//!
//! Sources: `portfolio/charge/domain/{ChargeTimeType,ChargeCalculationType,ChargeAppliesTo}`, `ChargePaymentMode`,
//! `portfolio/tax/service/TaxUtils.splitTax`, `portfolio/floatingrates/domain/FloatingRate` (`/Users/alejandro/fineract`).
#![forbid(unsafe_code)]

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_money::decimal;
use std::str::FromStr;
use time::Date;

/// Declares an enum whose variants carry the original's integer `code` and Java constant `name`.
macro_rules! coded_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $code:expr => $java:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            /// Numeric code, identical to the original enum.
            #[must_use] pub fn code(self) -> i32 { match self { $(Self::$variant => $code),+ } }
            /// Resolve from the numeric code.
            #[must_use] pub fn from_code(c: i32) -> Option<Self> {
                match c { $(c if c == $code => Some(Self::$variant),)+ _ => None }
            }
            /// The original Java enum-constant name.
            #[must_use] pub fn java_name(self) -> &'static str { match self { $(Self::$variant => $java),+ } }
            /// Resolve from the original Java enum-constant name.
            #[must_use] pub fn from_java_name(s: &str) -> Option<Self> {
                match s { $($java => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

coded_enum!(
    /// When a charge is triggered (`ChargeTimeType`). Note: the original has **no** `INVALID` constant and no code `4`
    /// — the value space is sparse (`from_code(0)` / `from_code(4)` correctly return `None`).
    ChargeTimeType {
        Disbursement = 1 => "DISBURSEMENT", SpecifiedDueDate = 2 => "SPECIFIED_DUE_DATE",
        SavingsActivation = 3 => "SAVINGS_ACTIVATION",
        WithdrawalFee = 5 => "WITHDRAWAL_FEE", AnnualFee = 6 => "ANNUAL_FEE", MonthlyFee = 7 => "MONTHLY_FEE",
        InstallmentFee = 8 => "INSTALLMENT_FEE", OverdueFees = 9 => "OVERDUE_FEES",
        OverdraftFee = 10 => "OVERDRAFT_FEE", WeeklyFee = 11 => "WEEKLY_FEE",
        TrancheDisbursement = 12 => "TRANCHE_DISBURSEMENT", ShareAccountActivate = 13 => "SHARE_ACCOUNT_ACTIVATE",
        SharePurchase = 14 => "SHARE_PURCHASE", ShareRedeem = 15 => "SHARE_REDEEM",
        SavingNoActivityFee = 16 => "SAVING_NO_ACTIVITY_FEE"
    }
);
coded_enum!(
    /// How a charge amount is computed (`ChargeCalculationType`).
    ChargeCalculationType {
        Invalid = 0 => "INVALID", Flat = 1 => "FLAT", PercentOfAmount = 2 => "PERCENT_OF_AMOUNT",
        PercentOfAmountAndInterest = 3 => "PERCENT_OF_AMOUNT_AND_INTEREST",
        PercentOfInterest = 4 => "PERCENT_OF_INTEREST", PercentOfDisbursementAmount = 5 => "PERCENT_OF_DISBURSEMENT_AMOUNT"
    }
);
coded_enum!(
    /// What a charge applies to (`ChargeAppliesTo`).
    ChargeAppliesTo {
        Invalid = 0 => "INVALID", Loan = 1 => "LOAN", Savings = 2 => "SAVINGS", Client = 3 => "CLIENT",
        Shares = 4 => "SHARES", WorkingCapitalLoan = 5 => "WORKING_CAPITAL_LOAN"
    }
);
coded_enum!(
    /// Charge payment mode (`ChargePaymentMode`).
    ChargePaymentMode { Regular = 0 => "REGULAR", AccountTransfer = 1 => "ACCOUNT_TRANSFER" }
);

impl ChargeCalculationType {
    /// `true` for the percentage-based calculation types (codes 2–5).
    #[must_use]
    pub fn is_percentage_based(self) -> bool {
        matches!(
            self,
            ChargeCalculationType::PercentOfAmount
                | ChargeCalculationType::PercentOfAmountAndInterest
                | ChargeCalculationType::PercentOfInterest
                | ChargeCalculationType::PercentOfDisbursementAmount
        )
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Tax (faithful to TaxUtils.splitTax — NOTE: original uses IEEE double; preserved for bit-parity)
// ---------------------------------------------------------------------------------------------------------------------

/// A tax component with its percentage and the effective date-range of its group mapping
/// (`TaxComponent` + `TaxGroupMappings.startDate/endDate`).
#[derive(Clone, Debug)]
pub struct TaxComponent {
    /// Resolved `getApplicablePercentage(date)`; `None` means the component is skipped (original returns `null`).
    pub percentage: Option<BigDecimal>,
    /// Inclusive start (`null` = open).
    pub start: Option<Date>,
    /// Inclusive end (`null` = open).
    pub end: Option<Date>,
}

impl TaxComponent {
    /// `TaxGroupMappings.occursOnDayFromAndUpToAndIncluding(date)` — inclusive range.
    #[must_use]
    pub fn occurs_on(&self, date: Date) -> bool {
        self.start.is_none_or(|s| date >= s) && self.end.is_none_or(|e| date <= e)
    }
}

/// `TaxUtils.splitTax`: per-applicable-component tax on `base` at `date`, rounded to `scale`.
///
/// **Faithful quirk:** the original computes `base.doubleValue() * pct.doubleValue() / 100.0` in IEEE `double`, then
/// `BigDecimal.valueOf(tax).setScale(scale, mode)`. Reproduced exactly (the one place floats are permitted in the
/// domain). Returns `(component_index, tax)` for each component whose mapping is active on `date`.
///
/// Like the original, a zero `base` is **not** short-circuited — each active mapping still yields a `0.00` entry.
/// Components whose resolved `percentage` is absent are skipped (mirrors `getApplicablePercentage == null`).
#[must_use]
pub fn split_tax(
    base: &BigDecimal,
    components: &[TaxComponent],
    date: Date,
    scale: i64,
    mode: RoundingMode,
) -> Vec<(usize, BigDecimal)> {
    components
        .iter()
        .enumerate()
        .filter(|(_, c)| c.occurs_on(date))
        .filter_map(|(i, c)| {
            c.percentage
                .as_ref()
                .map(|p| (i, tax_double(base, p, scale, mode)))
        })
        .collect()
}

/// Sum of a `split_tax` result.
#[must_use]
pub fn total_tax(split: &[(usize, BigDecimal)], scale: i64, mode: RoundingMode) -> BigDecimal {
    let sum = split
        .iter()
        .fold(BigDecimal::from(0), |acc, (_, t)| acc + t.clone());
    decimal::set_scale(&sum, scale, mode)
}

// Reproduces TaxUtils' IEEE-double computation: `BigDecimal.valueOf(amount.doubleValue()*pct.doubleValue()/100.0)`.
#[allow(clippy::float_arithmetic, clippy::disallowed_types)] // intentional: bit-parity with the original double-based tax
fn tax_double(base: &BigDecimal, pct: &BigDecimal, scale: i64, mode: RoundingMode) -> BigDecimal {
    let amount_val: f64 = base.to_string().parse().unwrap_or(0.0);
    let pct_val: f64 = pct.to_string().parse().unwrap_or(0.0);
    let tax = amount_val * pct_val / 100.0_f64; // <- original's double path (preserved quirk)
    // Java BigDecimal.valueOf(double) == new BigDecimal(Double.toString(double)); Rust's `{}` is the shortest round-trip.
    let as_decimal =
        BigDecimal::from_str(&format!("{tax}")).unwrap_or_else(|_| BigDecimal::from(0));
    decimal::set_scale(&as_decimal, scale, mode)
}

// ---------------------------------------------------------------------------------------------------------------------
// Floating rates (FloatingRate / FloatingRatePeriod)
// ---------------------------------------------------------------------------------------------------------------------

/// A floating-rate period (`FloatingRatePeriod`): effective from `from`. The original orders these by
/// `@OrderBy("fromDate,id")`; callers must pass `periods` in that order — the selection algorithm depends on it.
#[derive(Clone, Debug)]
pub struct FloatingRatePeriod {
    pub from: Date,
    pub rate: BigDecimal,
    pub is_differential_to_base_lending_rate: bool,
    pub is_active: bool,
}

/// Point-in-time rate for a **fixed** loan: the single period applicable on `date`. Equivalent to
/// `fetchInterestRates` with `floating = false` — the latest active period with `from ≤ date`, falling back to the
/// earliest active period when `date` precedes them all.
#[must_use]
pub fn effective_rate_period(
    periods: &[FloatingRatePeriod],
    date: Date,
) -> Option<&FloatingRatePeriod> {
    let on_or_before = periods
        .iter()
        .filter(|p| p.is_active && p.from <= date)
        .max_by_key(|p| p.from);
    on_or_before.or_else(|| periods.iter().find(|p| p.is_active))
}

/// Faithful port of `FloatingRate.fetchInterestRates` selection: given `start_date` and whether the loan's rate is
/// `floating`, returns the applicable active periods (in `@OrderBy("fromDate,id")` order).
///
/// - **Fixed** (`floating = false`): exactly one period — the one active at `start_date` (latest `from ≤ start_date`),
///   or the earliest period when `start_date` precedes them all.
/// - **Floating** (`floating = true`): the period active at `start_date` followed by every later period — the rate
///   time-series the schedule then applies as rates change.
///
/// Base-lending-rate composition (differential / basis points) is a higher-layer concern and is not applied here.
#[must_use]
pub fn fetch_interest_rates(
    periods: &[FloatingRatePeriod],
    start_date: Date,
    floating: bool,
) -> Vec<&FloatingRatePeriod> {
    let mut applicable: Vec<&FloatingRatePeriod> = Vec::new();
    let mut previous: Option<&FloatingRatePeriod> = None;
    let mut add_period_data = false;
    for period in periods.iter().filter(|p| p.is_active) {
        if applicable.is_empty() && start_date < period.from {
            if floating {
                add_period_data = true;
            }
            if let Some(prev) = previous {
                applicable.push(prev);
            } else if !add_period_data {
                applicable.push(period);
            }
        }
        if add_period_data {
            applicable.push(period);
        }
        previous = Some(period);
    }
    if applicable.is_empty()
        && let Some(prev) = previous
    {
        applicable.push(prev);
    }
    applicable
}
