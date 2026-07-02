//! Conformance: the accrual posting shapes against the rules documented (and read line-by-line) in
//! `AccrualBasedAccountingProcessorForLoan` — "Principal Repayment: Debits Fund Source and Credits Loan Portfolio;
//! Interest: Credits Receivable Interest; Fee: Credits Receivable Fees (or Income from Fees at disbursement);
//! Penalty: Credits Receivable Penalties" (src 1350-1363) plus the disbursement creator (src 1309). No Java unit
//! test exists for the processor (only server E2E features), so these lock the transcription and the double-entry
//! balance invariant.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_loan_accounting::{
    EntrySide, JournalEntry, LoanAccount, RepaymentPortions, TransferKind,
    journal_entries_for_disbursement, journal_entries_for_repayment,
};
use fc_money::Money;
use std::str::FromStr;

fn m(s: &str) -> Money {
    Money::of(2, RoundingMode::HalfEven, &BigDecimal::from_str(s).unwrap())
}

fn total(entries: &[JournalEntry], side: EntrySide) -> Money {
    entries
        .iter()
        .filter(|e| e.side == side)
        .fold(m("0"), |acc, e| acc.plus(&e.amount))
}

fn assert_entry(e: &JournalEntry, account: LoanAccount, side: EntrySide, amount: &str) {
    assert_eq!(e.account, account);
    assert_eq!(e.side, side);
    assert_eq!(e.amount.to_plain_string(), amount);
}

/// Disbursement (src 1309): Debit LOAN_PORTFOLIO 100, Credit FUND_SOURCE 100; with an overpayment refund the
/// principal debit shrinks and OVERPAYMENT takes the difference; transfers swap the credit account. Debits == credits.
#[test]
fn disbursement_shapes() {
    let plain = journal_entries_for_disbursement(&m("100"), &m("0"), TransferKind::None);
    assert_eq!(plain.len(), 2);
    assert_entry(
        &plain[0],
        LoanAccount::LoanPortfolio,
        EntrySide::Debit,
        "100.00",
    );
    assert_entry(
        &plain[1],
        LoanAccount::FundSource,
        EntrySide::Credit,
        "100.00",
    );

    let with_overpayment =
        journal_entries_for_disbursement(&m("100"), &m("30"), TransferKind::None);
    assert_eq!(with_overpayment.len(), 3);
    assert_entry(
        &with_overpayment[0],
        LoanAccount::LoanPortfolio,
        EntrySide::Debit,
        "70.00",
    );
    assert_entry(
        &with_overpayment[1],
        LoanAccount::Overpayment,
        EntrySide::Debit,
        "30.00",
    );
    assert_entry(
        &with_overpayment[2],
        LoanAccount::FundSource,
        EntrySide::Credit,
        "100.00",
    );
    assert_eq!(
        total(&with_overpayment, EntrySide::Debit).to_plain_string(),
        total(&with_overpayment, EntrySide::Credit).to_plain_string(),
        "double entry balances"
    );

    let transfer = journal_entries_for_disbursement(&m("100"), &m("0"), TransferKind::LoanToLoan);
    assert_entry(
        &transfer[1],
        LoanAccount::AssetTransfer,
        EntrySide::Credit,
        "100.00",
    );
    let account = journal_entries_for_disbursement(&m("100"), &m("0"), TransferKind::Account);
    assert_entry(
        &account[1],
        LoanAccount::LiabilityTransfer,
        EntrySide::Credit,
        "100.00",
    );
}

/// Repayment (src 1695): component credits in first-seen order then one FUND_SOURCE debit of the total; the
/// income-from-fee variant credits INCOME_FROM_FEES immediately and routes penalties to INCOME_FROM_PENALTIES.
/// Debits == credits in every variant.
#[test]
fn repayment_shapes() {
    let portions = RepaymentPortions {
        principal: m("16.43"),
        interest: m("0.58"),
        fees: m("5"),
        penalties: m("3"),
        overpayment: m("2"),
    };

    let receivable = journal_entries_for_repayment(&portions, false, TransferKind::None);
    assert_eq!(receivable.len(), 6);
    assert_entry(
        &receivable[0],
        LoanAccount::LoanPortfolio,
        EntrySide::Credit,
        "16.43",
    );
    assert_entry(
        &receivable[1],
        LoanAccount::InterestReceivable,
        EntrySide::Credit,
        "0.58",
    );
    assert_entry(
        &receivable[2],
        LoanAccount::FeesReceivable,
        EntrySide::Credit,
        "5.00",
    );
    assert_entry(
        &receivable[3],
        LoanAccount::PenaltiesReceivable,
        EntrySide::Credit,
        "3.00",
    );
    assert_entry(
        &receivable[4],
        LoanAccount::Overpayment,
        EntrySide::Credit,
        "2.00",
    );
    assert_entry(
        &receivable[5],
        LoanAccount::FundSource,
        EntrySide::Debit,
        "27.01",
    );
    assert_eq!(
        total(&receivable, EntrySide::Debit).to_plain_string(),
        total(&receivable, EntrySide::Credit).to_plain_string(),
        "double entry balances"
    );

    // Income-from-fee: the fee credit is immediate (first) and penalties go to income.
    let income = journal_entries_for_repayment(&portions, true, TransferKind::None);
    assert_eq!(income.len(), 6);
    assert_entry(
        &income[0],
        LoanAccount::IncomeFromFees,
        EntrySide::Credit,
        "5.00",
    );
    assert_entry(
        &income[1],
        LoanAccount::LoanPortfolio,
        EntrySide::Credit,
        "16.43",
    );
    assert_entry(
        &income[2],
        LoanAccount::InterestReceivable,
        EntrySide::Credit,
        "0.58",
    );
    assert_entry(
        &income[3],
        LoanAccount::IncomeFromPenalties,
        EntrySide::Credit,
        "3.00",
    );
    assert_entry(
        &income[4],
        LoanAccount::Overpayment,
        EntrySide::Credit,
        "2.00",
    );
    assert_entry(
        &income[5],
        LoanAccount::FundSource,
        EntrySide::Debit,
        "27.01",
    );

    // Zero components emit nothing; a principal-only repayment is a single credit/debit pair.
    let principal_only = journal_entries_for_repayment(
        &RepaymentPortions {
            principal: m("10"),
            interest: m("0"),
            fees: m("0"),
            penalties: m("0"),
            overpayment: m("0"),
        },
        false,
        TransferKind::Account,
    );
    assert_eq!(principal_only.len(), 2);
    assert_entry(
        &principal_only[0],
        LoanAccount::LoanPortfolio,
        EntrySide::Credit,
        "10.00",
    );
    assert_entry(
        &principal_only[1],
        LoanAccount::LiabilityTransfer,
        EntrySide::Debit,
        "10.00",
    );
}
