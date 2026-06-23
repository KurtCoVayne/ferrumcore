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
//! **Landed:** [`equal_amortization`] — the equal-amortization split used by re-age / re-amortize.
//!
//! **In progress (later slices):** the interest-schedule model (index-based redesign of the original's
//! `previous`-linked mutable graph), schedule-model generation + EMI, `getDueAmounts`, `payPrincipal` / `payInterest`,
//! `addDisbursement` / `addBalanceCorrection`, chargeback credit, re-age / re-amortize, capitalized income.
#![forbid(unsafe_code)]

pub mod equal_amortization;

pub use equal_amortization::{EqualAmortizationValues, calculate_equal_amortization_values};
