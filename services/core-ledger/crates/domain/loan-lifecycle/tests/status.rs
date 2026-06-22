//! Unit coverage for `LoanStatus` codes + predicates (faithful to the original `LoanStatus`).
#![allow(clippy::unwrap_used)] // test code

use fc_loan_lifecycle::LoanStatus::{
    Active, Approved, ClosedObligationsMet, ClosedRescheduleOutstandingAmount, ClosedWrittenOff,
    Overpaid, SubmittedAndPendingApproval, TransferInProgress, TransferOnHold,
};

#[test]
fn codes_match_original() {
    assert_eq!(SubmittedAndPendingApproval.code(), 100);
    assert_eq!(Approved.code(), 200);
    assert_eq!(Active.code(), 300);
    assert_eq!(TransferInProgress.code(), 303);
    assert_eq!(TransferOnHold.code(), 304);
    assert_eq!(ClosedObligationsMet.code(), 600);
    assert_eq!(ClosedWrittenOff.code(), 601);
    assert_eq!(ClosedRescheduleOutstandingAmount.code(), 602);
    assert_eq!(Overpaid.code(), 700);
}

#[test]
fn predicates_match_original() {
    assert!(Active.is_active());
    assert!(Overpaid.is_overpaid());
    // isClosed() = obligations-met | written-off | reschedule-outstanding
    assert!(ClosedObligationsMet.is_closed());
    assert!(ClosedWrittenOff.is_closed());
    assert!(ClosedRescheduleOutstandingAmount.is_closed());
    assert!(!Active.is_closed());
    assert!(!Overpaid.is_closed());
    // isActiveOrAwaitingApprovalOrDisbursal()
    assert!(Approved.is_active_or_awaiting_approval_or_disbursal());
    assert!(SubmittedAndPendingApproval.is_active_or_awaiting_approval_or_disbursal());
    assert!(Active.is_active_or_awaiting_approval_or_disbursal());
    assert!(!ClosedObligationsMet.is_active_or_awaiting_approval_or_disbursal());
    // isUnderTransfer()
    assert!(TransferInProgress.is_under_transfer());
    assert!(TransferOnHold.is_under_transfer());
    assert!(!Active.is_under_transfer());
}
