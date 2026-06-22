//! # fc-loan-lifecycle
//!
//! Faithful port of the original `DefaultLoanLifecycleStateMachine.getNextStatus(event, loan)` (BI-1 / invariant I2).
//! Source: `fineract-loan/.../portfolio/loanaccount/domain/DefaultLoanLifecycleStateMachine.java` + `LoanStatus.java`,
//! validated by `DefaultLoanLifecycleStateMachineTest` (confirmed GREEN 2026-06-21). Pure: no IO, no framework.
//!
//! A few transitions depend on loan *financial* predicates (overpaid / repaid-in-full / outstanding-zero) rather than
//! state alone; those are passed explicitly in [`Ctx`] (the orchestration supplies them — the state machine stays pure).
#![forbid(unsafe_code)]

/// Loan status (code matches the original `LoanStatus` enum values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanStatus {
    Invalid,
    SubmittedAndPendingApproval,
    Approved,
    Active,
    TransferInProgress,
    TransferOnHold,
    WithdrawnByClient,
    Rejected,
    ClosedObligationsMet,
    ClosedWrittenOff,
    ClosedRescheduleOutstandingAmount,
    Overpaid,
}

impl LoanStatus {
    /// Numeric code, identical to the original enum.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            LoanStatus::Invalid => 0,
            LoanStatus::SubmittedAndPendingApproval => 100,
            LoanStatus::Approved => 200,
            LoanStatus::Active => 300,
            LoanStatus::TransferInProgress => 303,
            LoanStatus::TransferOnHold => 304,
            LoanStatus::WithdrawnByClient => 400,
            LoanStatus::Rejected => 500,
            LoanStatus::ClosedObligationsMet => 600,
            LoanStatus::ClosedWrittenOff => 601,
            LoanStatus::ClosedRescheduleOutstandingAmount => 602,
            LoanStatus::Overpaid => 700,
        }
    }

    /// Parse from the original Java enum-constant name (e.g. `SUBMITTED_AND_PENDING_APPROVAL`).
    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "INVALID" => LoanStatus::Invalid,
            "SUBMITTED_AND_PENDING_APPROVAL" => LoanStatus::SubmittedAndPendingApproval,
            "APPROVED" => LoanStatus::Approved,
            "ACTIVE" => LoanStatus::Active,
            "TRANSFER_IN_PROGRESS" => LoanStatus::TransferInProgress,
            "TRANSFER_ON_HOLD" => LoanStatus::TransferOnHold,
            "WITHDRAWN_BY_CLIENT" => LoanStatus::WithdrawnByClient,
            "REJECTED" => LoanStatus::Rejected,
            "CLOSED_OBLIGATIONS_MET" => LoanStatus::ClosedObligationsMet,
            "CLOSED_WRITTEN_OFF" => LoanStatus::ClosedWrittenOff,
            "CLOSED_RESCHEDULE_OUTSTANDING_AMOUNT" => LoanStatus::ClosedRescheduleOutstandingAmount,
            "OVERPAID" => LoanStatus::Overpaid,
            _ => return None,
        })
    }
}

/// Loan lifecycle event (subset of the original `LoanEvent` that the state machine acts on; others are no-ops).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanEvent {
    LoanCreated,
    LoanRejected,
    LoanApproved,
    LoanWithdrawn,
    LoanApprovalUndo,
    LoanDisbursed,
    LoanDisbursalUndo,
    LoanRepaymentOrWaiver,
    LoanChargePayment,
    LoanChargeback,
    RepaidInFull,
    WriteOffOutstanding,
    WriteOffOutstandingUndo,
    LoanReschedule,
    LoanOverpayment,
    LoanAdjustTransaction,
    LoanInitiateTransfer,
    LoanRejectTransfer,
    LoanWithdrawTransfer,
    LoanCompleteTransfer,
    LoanCreditBalanceRefund,
    LoanChargeAdded,
    LoanChargeAdjustment,
    /// Any original `LoanEvent` not listed above — no status transition in this machine.
    Other,
}

impl LoanEvent {
    /// Parse from the original Java enum-constant name; unknown/no-op events map to [`LoanEvent::Other`].
    #[must_use]
    pub fn from_name(s: &str) -> Self {
        match s {
            "LOAN_CREATED" => LoanEvent::LoanCreated,
            "LOAN_REJECTED" => LoanEvent::LoanRejected,
            "LOAN_APPROVED" => LoanEvent::LoanApproved,
            "LOAN_WITHDRAWN" => LoanEvent::LoanWithdrawn,
            "LOAN_APPROVAL_UNDO" => LoanEvent::LoanApprovalUndo,
            "LOAN_DISBURSED" => LoanEvent::LoanDisbursed,
            "LOAN_DISBURSAL_UNDO" => LoanEvent::LoanDisbursalUndo,
            "LOAN_REPAYMENT_OR_WAIVER" => LoanEvent::LoanRepaymentOrWaiver,
            "LOAN_CHARGE_PAYMENT" => LoanEvent::LoanChargePayment,
            "LOAN_CHARGEBACK" => LoanEvent::LoanChargeback,
            "REPAID_IN_FULL" => LoanEvent::RepaidInFull,
            "WRITE_OFF_OUTSTANDING" => LoanEvent::WriteOffOutstanding,
            "WRITE_OFF_OUTSTANDING_UNDO" => LoanEvent::WriteOffOutstandingUndo,
            "LOAN_RESCHEDULE" => LoanEvent::LoanReschedule,
            "LOAN_OVERPAYMENT" => LoanEvent::LoanOverpayment,
            "LOAN_ADJUST_TRANSACTION" => LoanEvent::LoanAdjustTransaction,
            "LOAN_INITIATE_TRANSFER" => LoanEvent::LoanInitiateTransfer,
            "LOAN_REJECT_TRANSFER" => LoanEvent::LoanRejectTransfer,
            "LOAN_WITHDRAW_TRANSFER" => LoanEvent::LoanWithdrawTransfer,
            "LOAN_COMPLETE_TRANSFER" => LoanEvent::LoanCompleteTransfer,
            "LOAN_CREDIT_BALANCE_REFUND" => LoanEvent::LoanCreditBalanceRefund,
            "LOAN_CHARGE_ADDED" => LoanEvent::LoanChargeAdded,
            "LOAN_CHARGE_ADJUSTMENT" => LoanEvent::LoanChargeAdjustment,
            _ => LoanEvent::Other,
        }
    }
}

/// Financial predicates the original reads from the `Loan` for the conditional transitions (supplied by orchestration).
#[derive(Clone, Copy, Debug, Default)]
pub struct Ctx {
    pub overpaid: bool,
    pub total_overpaid_zero: bool,
    pub outstanding_zero: bool,
    pub repaid_in_full: bool,
}

/// The transition function — faithful 1:1 reproduction of `getNextStatus`. `None` = no transition (illegal/no-op).
#[must_use]
pub fn next_status(event: LoanEvent, from: Option<LoanStatus>, ctx: Ctx) -> Option<LoanStatus> {
    use LoanEvent as E;
    use LoanStatus as S;

    if event == E::LoanCreated && from.is_none() {
        return Some(S::SubmittedAndPendingApproval);
    }
    let from = from?; // non-creation events with no from-state never transition
    let in_set = |set: &[S]| set.contains(&from);

    match event {
        E::LoanRejected => (from == S::SubmittedAndPendingApproval).then_some(S::Rejected),
        E::LoanApproved => (from == S::SubmittedAndPendingApproval).then_some(S::Approved),
        E::LoanWithdrawn => {
            (from == S::SubmittedAndPendingApproval).then_some(S::WithdrawnByClient)
        }
        E::LoanDisbursed => {
            if in_set(&[S::Approved, S::ClosedObligationsMet]) {
                Some(S::Active)
            } else if from == S::Overpaid && ctx.total_overpaid_zero {
                Some(if ctx.outstanding_zero {
                    S::ClosedObligationsMet
                } else {
                    S::Active
                })
            } else {
                None
            }
        }
        E::LoanApprovalUndo => (from == S::Approved).then_some(S::SubmittedAndPendingApproval),
        E::LoanDisbursalUndo => (from == S::Active).then_some(S::Approved),
        E::LoanChargePayment | E::LoanRepaymentOrWaiver | E::LoanChargeback => {
            in_set(&[S::ClosedObligationsMet, S::Overpaid]).then_some(S::Active)
        }
        E::RepaidInFull => in_set(&[S::Active, S::Overpaid]).then_some(S::ClosedObligationsMet),
        E::WriteOffOutstanding => (from == S::Active).then_some(S::ClosedWrittenOff),
        E::LoanReschedule => (from == S::Active).then_some(S::ClosedRescheduleOutstandingAmount),
        E::LoanOverpayment => in_set(&[S::ClosedObligationsMet, S::Active]).then_some(S::Overpaid),
        E::LoanAdjustTransaction => in_set(&[
            S::ClosedObligationsMet,
            S::ClosedWrittenOff,
            S::ClosedRescheduleOutstandingAmount,
        ])
        .then_some(if ctx.overpaid { S::Overpaid } else { S::Active }),
        E::LoanInitiateTransfer => Some(S::TransferInProgress),
        E::LoanRejectTransfer => (from == S::TransferInProgress).then_some(S::TransferOnHold),
        E::LoanWithdrawTransfer => (from == S::TransferInProgress).then_some(S::Active),
        E::LoanCompleteTransfer => (from == S::TransferInProgress).then_some(if ctx.overpaid {
            S::Overpaid
        } else if ctx.repaid_in_full {
            S::ClosedObligationsMet
        } else {
            S::Active
        }),
        E::WriteOffOutstandingUndo => (from == S::ClosedWrittenOff).then_some(S::Active),
        E::LoanCreditBalanceRefund => (from == S::Overpaid).then_some(S::ClosedObligationsMet),
        E::LoanChargeAdded => (from == S::ClosedObligationsMet).then_some(S::Active),
        E::LoanChargeAdjustment => (from == S::ClosedObligationsMet).then_some(S::Overpaid),
        E::LoanCreated | E::Other => None,
    }
}
