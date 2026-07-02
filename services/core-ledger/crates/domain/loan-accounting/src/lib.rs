//! `fc-loan-accounting` — the loan-transaction → journal-entry mapping (S4), a faithful port of the **accrual**
//! posting rules in `AccrualBasedAccountingProcessorForLoan`
//! (`fineract-provider/.../accounting/journalentry/service/AccrualBasedAccountingProcessorForLoan.java`).
//!
//! Scope (the foundational entry shapes every other creator composes):
//! - **Disbursement** (src 1309): Debit LOAN_PORTFOLIO for the principal portion (amount − overpayment), Debit
//!   OVERPAYMENT for the refunded overpayment portion, one Credit for the full amount against FUND_SOURCE — or the
//!   ASSET_TRANSFER / LIABILITY_TRANSFER financial-activity account for loan-to-loan / account transfers.
//! - **Repayment** (src 1695, the non-charged-off `createJournalEntriesForLoanRepayments`): per-component credits
//!   accumulated per account in first-seen order — principal → LOAN_PORTFOLIO, interest → INTEREST_RECEIVABLE, fees →
//!   FEES_RECEIVABLE (or an immediate INCOME_FROM_FEES credit when the payment recognizes fee income directly, e.g.
//!   repayment-at-disbursement charges), penalties → PENALTIES_RECEIVABLE (INCOME_FROM_PENALTIES for direct income),
//!   overpayment → OVERPAYMENT — followed by a **single debit** of the total against FUND_SOURCE (or the transfer
//!   accounts).
//!
//! Out of scope (follow-on creators, each its own port): the charged-off variants (`GLAccountBalanceHolder` netting),
//! goodwill-credit debit splitting, charge-refund extra pair, tax-liability splitting on fee income, write-offs,
//! chargeback, charge-off, capitalized-income / buy-down-fee amortization entries, interest-payment-waiver, accrual
//! activity, and the cash-basis processor. Account resolution here is by **account type** — Java resolves each type
//! to a concrete GL account via the product's financial mapping (and merges entries landing on the same GL account);
//! with distinct product mappings the two are identical.
#![forbid(unsafe_code)]

use fc_money::Money;

/// `AccrualAccountsForLoan` (subset) + the transfer `FinancialActivity` accounts — the account *types* the core entry
/// shapes post to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoanAccount {
    /// `AccrualAccountsForLoan.FUND_SOURCE`.
    FundSource,
    /// `AccrualAccountsForLoan.LOAN_PORTFOLIO`.
    LoanPortfolio,
    /// `AccrualAccountsForLoan.INTEREST_RECEIVABLE`.
    InterestReceivable,
    /// `AccrualAccountsForLoan.FEES_RECEIVABLE`.
    FeesReceivable,
    /// `AccrualAccountsForLoan.PENALTIES_RECEIVABLE`.
    PenaltiesReceivable,
    /// `AccrualAccountsForLoan.INCOME_FROM_FEES`.
    IncomeFromFees,
    /// `AccrualAccountsForLoan.INCOME_FROM_PENALTIES`.
    IncomeFromPenalties,
    /// `AccrualAccountsForLoan.OVERPAYMENT`.
    Overpayment,
    /// `FinancialActivity.ASSET_TRANSFER`.
    AssetTransfer,
    /// `FinancialActivity.LIABILITY_TRANSFER`.
    LiabilityTransfer,
}

/// Which side of the ledger an entry posts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntrySide {
    Debit,
    Credit,
}

/// One journal entry: an amount posted to an account type on one side.
#[derive(Clone, Debug)]
pub struct JournalEntry {
    pub account: LoanAccount,
    pub side: EntrySide,
    pub amount: Money,
}

/// How the money moved — determines the counter-account (`isLoanToLoanTransfer` / `isAccountTransfer` / cash).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    /// A regular payment — FUND_SOURCE is the counter-account.
    None,
    /// `isLoanToLoanTransfer()` — the ASSET_TRANSFER financial activity.
    LoanToLoan,
    /// `isAccountTransfer()` — the LIABILITY_TRANSFER financial activity.
    Account,
}

impl TransferKind {
    /// The counter-account the transfer kind selects (src 1336-1347 / 1839-1858).
    #[must_use]
    fn counter_account(self) -> LoanAccount {
        match self {
            TransferKind::None => LoanAccount::FundSource,
            TransferKind::LoanToLoan => LoanAccount::AssetTransfer,
            TransferKind::Account => LoanAccount::LiabilityTransfer,
        }
    }
}

/// `createJournalEntriesForDisbursements` (src 1309): Debit LOAN_PORTFOLIO with the principal portion
/// (`amount − overpayment`), Debit OVERPAYMENT with the refunded overpayment, Credit the full amount on the
/// counter-account.
#[must_use]
pub fn journal_entries_for_disbursement(
    amount: &Money,
    overpayment: &Money,
    transfer: TransferKind,
) -> Vec<JournalEntry> {
    let mut entries = Vec::new();
    let principal = amount.minus(overpayment);
    if principal.is_greater_than_zero() {
        entries.push(JournalEntry {
            account: LoanAccount::LoanPortfolio,
            side: EntrySide::Debit,
            amount: principal,
        });
    }
    if overpayment.is_greater_than_zero() {
        entries.push(JournalEntry {
            account: LoanAccount::Overpayment,
            side: EntrySide::Debit,
            amount: overpayment.clone(),
        });
    }
    if amount.is_greater_than_zero() {
        entries.push(JournalEntry {
            account: transfer.counter_account(),
            side: EntrySide::Credit,
            amount: amount.clone(),
        });
    }
    entries
}

/// The component portions of a repayment-family transaction (`LoanTransactionDTO` subset).
#[derive(Clone, Debug)]
pub struct RepaymentPortions {
    pub principal: Money,
    pub interest: Money,
    pub fees: Money,
    pub penalties: Money,
    pub overpayment: Money,
}

/// `createJournalEntriesForLoanRepayments` (src 1695) for the non-charged-off, non-goodwill, non-charge-refund core:
/// per-component credits accumulated per account in first-seen order (principal → LOAN_PORTFOLIO, interest →
/// INTEREST_RECEIVABLE, fees → FEES_RECEIVABLE or an immediate INCOME_FROM_FEES credit when `is_income_from_fee`,
/// penalties → PENALTIES_RECEIVABLE or INCOME_FROM_PENALTIES, overpayment → OVERPAYMENT), then a single debit of the
/// total on the counter-account. Java emits the direct fee-income credit inside the fee branch — *before* the
/// accumulated map's credits — and that order is preserved here.
#[must_use]
pub fn journal_entries_for_repayment(
    portions: &RepaymentPortions,
    is_income_from_fee: bool,
    transfer: TransferKind,
) -> Vec<JournalEntry> {
    let mut total_debit = portions.principal.minus(&portions.principal); // zero at the portions' scale
    let mut account_map: Vec<(LoanAccount, Money)> = Vec::new();
    let mut immediate_credits: Vec<JournalEntry> = Vec::new();
    let add = |map: &mut Vec<(LoanAccount, Money)>, account: LoanAccount, amount: &Money| match map
        .iter_mut()
        .find(|(a, _)| *a == account)
    {
        Some((_, acc)) => *acc = acc.plus(amount),
        None => map.push((account, amount.clone())),
    };

    if portions.principal.is_greater_than_zero() {
        total_debit = total_debit.plus(&portions.principal);
        add(
            &mut account_map,
            LoanAccount::LoanPortfolio,
            &portions.principal,
        );
    }
    if portions.interest.is_greater_than_zero() {
        total_debit = total_debit.plus(&portions.interest);
        add(
            &mut account_map,
            LoanAccount::InterestReceivable,
            &portions.interest,
        );
    }
    if portions.fees.is_greater_than_zero() {
        total_debit = total_debit.plus(&portions.fees);
        if is_income_from_fee {
            // Direct income recognition (repayment-at-disbursement charges): the credit is emitted immediately.
            immediate_credits.push(JournalEntry {
                account: LoanAccount::IncomeFromFees,
                side: EntrySide::Credit,
                amount: portions.fees.clone(),
            });
        } else {
            add(
                &mut account_map,
                LoanAccount::FeesReceivable,
                &portions.fees,
            );
        }
    }
    if portions.penalties.is_greater_than_zero() {
        total_debit = total_debit.plus(&portions.penalties);
        let account = if is_income_from_fee {
            LoanAccount::IncomeFromPenalties
        } else {
            LoanAccount::PenaltiesReceivable
        };
        add(&mut account_map, account, &portions.penalties);
    }
    if portions.overpayment.is_greater_than_zero() {
        total_debit = total_debit.plus(&portions.overpayment);
        add(
            &mut account_map,
            LoanAccount::Overpayment,
            &portions.overpayment,
        );
    }

    let mut entries = immediate_credits;
    for (account, amount) in account_map {
        if amount.is_greater_than_zero() {
            entries.push(JournalEntry {
                account,
                side: EntrySide::Credit,
                amount,
            });
        }
    }
    if total_debit.is_greater_than_zero() {
        entries.push(JournalEntry {
            account: transfer.counter_account(),
            side: EntrySide::Debit,
            amount: total_debit,
        });
    }
    entries
}
