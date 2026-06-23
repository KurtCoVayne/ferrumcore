//! Shared value types for payment allocation: the transaction view (`Txn`/`TxnKind`), the per-installment result
//! ([`Allocation`]), and the pending-charge view used by in-advance due-charge ordering ([`PendingCharge`]).

use bigdecimal::BigDecimal;
use fc_money::Money;
use time::Date;

/// What kind of money movement a transaction represents, mirroring the `loanTransaction.is*` predicates the original
/// processors branch on. Determines whether the strategy's repayment ordering runs or a fixed component is touched.
#[derive(Clone, Debug)]
pub enum TxnKind {
    /// An ordinary repayment — the strategy's component ordering applies.
    Repayment,
    /// `isInterestWaiver()` — waive interest up to the transaction amount.
    InterestWaiver,
    /// `isChargesWaiver()` — waive the transaction's penalty then fee portions (carried on the transaction).
    ChargesWaiver {
        penalty_portion: BigDecimal,
        fee_portion: BigDecimal,
    },
    /// `isChargePayment()` — pay exactly one charge bucket (penalty if `is_penalty`, else fee).
    ChargePayment { is_penalty: bool },
}

/// A transaction as seen by allocation: its amount, value date, and kind.
#[derive(Clone, Debug)]
pub struct Txn {
    pub amount: Money,
    pub date: Date,
    pub kind: TxnKind,
}

impl Txn {
    /// An ordinary repayment for `amount` on `date`.
    #[must_use]
    pub fn repayment(date: Date, amount: Money) -> Self {
        Self {
            amount,
            date,
            kind: TxnKind::Repayment,
        }
    }
}

/// The outcome of allocating one transaction against one installment: the four component portions consumed (the
/// original's `loanTransaction.updateComponents(principal, interest, fee, penalty)` tuple) plus the amount left
/// unprocessed (the handler's return value).
#[derive(Clone, Debug)]
pub struct Allocation {
    pub principal: Money,
    pub interest: Money,
    pub fee: Money,
    pub penalty: Money,
    pub unprocessed: Money,
}

/// A pending loan charge, as seen by the in-advance due-charge ordering (subset of `LoanCharge` the processor reads).
#[derive(Clone, Debug)]
pub struct PendingCharge {
    /// `getEffectiveDueDate()` — `None` means "no due date" (always counts as due).
    pub effective_due_date: Option<Date>,
    /// `isPenaltyCharge()` — penalty vs fee.
    pub is_penalty: bool,
    /// `getAmount(currency)`.
    pub amount: BigDecimal,
    /// `isActive()`.
    pub is_active: bool,
    /// `isChargePending()`.
    pub is_pending: bool,
}
