//! # fc-loan-allocation
//!
//! Loan payment **allocation** kernel (contract C2) — faithful port of the legacy
//! `LoanRepaymentScheduleTransactionProcessor` family (`fineract-loan/.../transactionprocessor/`). Pure: no IO, no
//! framework, no clock.
//!
//! - [`Installment`] ports the per-component arithmetic of `LoanRepaymentScheduleInstallment`: each component
//!   (principal / interest / fee / penalty) tracks a *charged* amount and what's *accounted for* (paid / waived /
//!   written off); `outstanding = charged − accounted`. The `pay_*` / `waive_*` / `unpay_*` atoms move money between
//!   those buckets exactly as the original does.
//! - [`strategy`] holds the legacy strategies. Each implements only the *repayment* component ordering ([`Strategy`]);
//!   the kind-aware entry points ([`allocate_on_time`](strategy::allocate_on_time) etc.) layer on the waiver /
//!   charge-payment / refund branches every original processor shares.
//! - [`Txn`] / [`TxnKind`] / [`Allocation`] / [`PendingCharge`] are the shared value types.
//!
//! - [`driver`] ports the multi-installment walk (`AbstractLoanRepaymentScheduleTransactionProcessor.processTransaction`
//!   / the default branch of `processLatestTransaction`): which installment(s) a transaction lands on, the in-advance /
//!   on-time / late bucketing per installment (via the [`Strategy`] timing hooks), carry-forward of the unprocessed
//!   remainder, and the final overpayment.
//! - [`advanced`] ports a bounded, test-backed slice of the progressive `AdvancedPaymentScheduleTransactionProcessor`
//!   (charge payment + chargeback credited-principal).
//!
//! Proven by the per-strategy conformance suites in `tests/`, which replay the original VERIFIED unit tests
//! (`*LoanRepaymentScheduleTransactionProcessorTest`) and assert the exact
//! `updateComponents(principal, interest, fee, penalty)` tuple plus the unprocessed remainder.
//!
//! **Out of scope (later slices):** the rest of the 4296-line `AdvancedPaymentScheduleTransactionProcessor` (interest
//! recalculation, re-age/re-amortize, prior-chargeback adjustment). Installment side-state that does not affect the
//! allocation outcome (`obligationsMet`, advance/late running totals) is intentionally not modeled.
#![forbid(unsafe_code)]

pub mod advanced;
pub mod capitalized_income;
pub mod charges;
pub mod driver;
pub mod emi_adapter;
pub mod installment;
pub mod model;
pub mod strategy;

pub use emi_adapter::ProgressiveEmiCalculator;
pub use installment::Installment;
pub use model::{Allocation, PendingCharge, Txn, TxnKind};
pub use strategy::Strategy;
