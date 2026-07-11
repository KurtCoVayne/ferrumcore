# fc-loan-journal

The **persistence and reconstruction layer** for progressive-loan interest schedules
(ADR-0026). It is the application-layer projection over the serde-free EMI kernel
(`fc-loan-emi`): a serializable, append-only **operation log** per loan, plus deterministic
**replay** back into the interest-schedule model, plus a pre-replay **validation boundary**.

## Why an operation log, not a model blob

Apache Fineract persists the whole internal `ProgressiveLoanInterestScheduleModel` as a Gson
JSON blob and reloads it. That blob's shape *is* the internal object graph — `previous`
back-refs, memo caches, interest periods, rate factors — so it is schema-fragile across
releases, and it is exactly the representation this port deliberately restructured
(index-based, no back-pointers, always-fresh getters).

FerrumCore instead persists the **ordered sequence of engine operations** a loan underwent
(schedule init + disbursement / capitalized-income / pay / credit / balance-correction /
rate-change / due-date-change / add-periods / interest-pause / re-age) and **recomputes** the
model by replaying that log through the kernel on load. Because the kernel is a pure function
of the log (no wall-clock — ADR-0009; no interior mutability), replay is deterministic and
**byte-exact**; the log is also the audit trail (aligns with ADR-0018/0022). Snapshotting a
replayed model plus a tail of events is an available later optimization, deferred until
profiling shows need.

`serde` lives only in this crate; the hot domain crates stay serde-free and value-semantic
(Part-4 invariant + thread-safety).

## Encoding

Money is an **exact-decimal string** and dates are **ISO-8601 `YYYY-MM-DD`** (ADR-0005), so the
serialized form never contains a binary float and is stable across releases. The rounding mode
is fixed to banker's rounding (`HALF_EVEN`) — the only mode the system uses.

## API

```rust
use fc_loan_journal::{LoanJournal, ScheduleInit, LoanOp};

let journal = LoanJournal { init: ScheduleInit::Declining { /* … */ }, ops: vec![/* … */] };

// Boundary: reject incoherent input BEFORE the kernel (empty periods, non-positive principal,
// negative amounts, zero-installment re-age — the cases the kernel would silently clamp).
let issues = journal.validate();
assert!(issues.is_empty());

// Reconstruct the interest-schedule model by replaying the log.
let model = journal.replay()?;      // -> fc_loan_emi::recalc::Model
```

`validate()` returns every `ValidationIssue` found (empty = coherent). `replay()` returns a
typed `JournalError` on a malformed field rather than panicking (deny-panic at the boundary).

## Runnable host (example)

```
cargo run -p fc-loan-journal --example replay_cli -- <journal.json>
```

Reads a persisted journal, validates, replays, and prints the reconstructed schedule as JSON.
Exit 0 = ok; 2 = validation issues (listed on stderr); 1 = bad input / IO / parse. This is the
shape the production gRPC transport adapter (ADR-0025) wraps.

## Guarantees (tests)

- `replay == live` **byte-identical** for the payoff and FULL_INTEREST re-age golden sequences.
- lossless JSON round-trip (float-free).
- every incoherent-input case is surfaced by `validate()`; a malformed log is a typed error.
