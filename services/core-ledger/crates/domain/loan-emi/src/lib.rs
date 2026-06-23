//! # fc-loan-emi
//!
//! Progressive loan **EMI / interest-schedule engine** — faithful port of the original
//! `ProgressiveEMICalculator` (`fineract-progressive-loan/.../loanproduct/calc/`) and its interest-schedule model
//! (`ProgressiveLoanInterestScheduleModel` / `RepaymentPeriod` / `InterestPeriod`). Pure: no IO, no framework, no clock.
//!
//! This is the engine the `fc-loan-allocation` Advanced processor delegates to via its `EmiCalculator` seam (re-age /
//! re-amortize / capitalized-income / interest recalculation). It is a large subsystem ported incrementally, each
//! piece verified against the original's VERIFIED tests (`ProgressiveEMICalculatorTest`, run on JDK 21 to capture the
//! golden behavior).
//!
//! **Landed:** [`math`] — a faithful Java `MathContext` ([`Mc`]) with no-pad rounding + `divide` preferred-scale;
//! [`equal_amortization`] — the equal-amortization split used by re-age / re-amortize; [`rate_factor`] — the
//! significant-figure rate-factor (`rateFactorByRepaymentEveryMonth`) and annuity `fnValue` primitives;
//! [`interest_period`] — the declining-balance per-period interest. All golden-verified against
//! `ProgressiveEMICalculatorTest`.
//!
//! **In progress (later slices):** the interest-schedule model (index-based redesign of the original's
//! `previous`-linked mutable graph), schedule-model generation + EMI, `getDueAmounts`, `payPrincipal` / `payInterest`,
//! `addDisbursement` / `addBalanceCorrection`, chargeback credit, re-age / re-amortize, capitalized income.
#![forbid(unsafe_code)]

pub mod equal_amortization;
pub mod interest_period;
pub mod math;
pub mod rate_factor;
pub mod schedule;

pub use math::Mc;
pub use schedule::{
    DayCountKind, Model, PeriodDueDetails, RepaymentPeriod, generate_declining_balance,
};

pub use equal_amortization::{
    EqualAmortizationValues, calculate_adjusted_equal_amortization_values,
    calculate_equal_amortization_values,
};
pub use rate_factor::{
    fn_value, rate_factor_by_repayment_every_month, rate_factor_by_repayment_period,
};
