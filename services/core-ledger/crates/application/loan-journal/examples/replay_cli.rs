//! A minimal **runnable host** proving the persistence pipeline end-to-end as a real process:
//! read a persisted loan journal (JSON) → validate at the boundary → replay through the kernel → project the
//! reconstructed schedule to JSON on stdout. This is the shape the production gRPC transport adapter wraps (ADR-0025);
//! here it runs as a CLI so the whole path is executable and verifiable without the mTLS/gateway stack.
//!
//! Usage: `cargo run -p fc-loan-journal --example replay_cli -- <journal.json>`
//! Exit codes: 0 = ok (schedule on stdout); 2 = validation issues (listed on stderr); 1 = bad input / IO / parse.
#![allow(clippy::print_stdout, clippy::print_stderr)] // this is a CLI

use fc_loan_journal::LoanJournal;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: replay_cli <journal.json>");
            return Ok(ExitCode::from(1));
        }
    };
    let raw = std::fs::read_to_string(&path)?;
    let journal: LoanJournal = serde_json::from_str(&raw)?;

    // Boundary: reject incoherent input explicitly before it reaches the kernel.
    let issues = journal.validate();
    if !issues.is_empty() {
        eprintln!("{} validation issue(s):", issues.len());
        for issue in &issues {
            eprintln!("  - {issue:?}");
        }
        return Ok(ExitCode::from(2));
    }

    // Reconstruct the interest-schedule model from the log and project it to a flat schedule.
    let model = journal.replay()?;
    let mut out = String::from("{\n  \"schedule\": [\n");
    for i in 0..model.periods.len() {
        let rp = &model.periods[i];
        out.push_str(&format!(
            "    {{ \"from\": \"{}\", \"due\": \"{}\", \"emi\": \"{}\", \"due_principal\": \"{}\", \"due_interest\": \"{}\", \"outstanding\": \"{}\", \"fully_paid\": {} }}{}\n",
            fc_loan_journal::format_date(rp.from),
            fc_loan_journal::format_date(rp.due),
            model.period_emi(i).to_plain_string(),
            model.period_due_principal(i).to_plain_string(),
            model.period_due_interest(i).to_plain_string(),
            model.period_outstanding(i).to_plain_string(),
            model.period_fully_paid(i),
            if i + 1 < model.periods.len() { "," } else { "" },
        ));
    }
    out.push_str("  ]\n}");
    println!("{out}");
    Ok(ExitCode::SUCCESS)
}
