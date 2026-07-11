# Core-Ledger Loan Kernel — Production Readiness

Status snapshot from the FerrumCore ↔ Fineract gap-closure effort. Two phases: **Phase 1**
closes parity/quality gaps inside the golden-verified math kernel; **Phase 2** builds the
productionization layers around it.

## Verdict

The **math kernel is production-grade as a library** within its verified scope: golden-verified
against Fineract (110-test EMI suite + 13-test processor suite + TSV corpora + JVM-replay
goldens), panic-free (`deny` unwrap/expect/panic/float, `forbid` unsafe), thread-safe by
construction (value-semantic, no interior mutability), ~13× faster than Java, CI-gated
(fmt/clippy/test/deny over the whole workspace). The surrounding **service layers** are being
built out (Phase 2); a full deployable system additionally needs the gRPC transport host,
observability wiring, and the automated dual-run parity gate.

## Phase 1 — kernel parity & quality (in progress)

- **Divergence register documented** — the three real Rust↔Java divergences (re-amortization
  empty-future skip, re-age `try_add` totalization, rest-frequency type-narrowing) are
  documented at their call sites; dead-config flags (`future_unrecognized_interest`,
  `is_interest_moved_upward`, the pub-field-only `Model` config) are annotated.
- **`forbid(unsafe_code)`** added to the one crate that lacked it (`fc-loan-accounting`).
- **Charge dimension** — the largest parity item — landed core + walk wiring: charge-payment
  distribution (`charges.rs`: earliest-due-first pay, latest-paid-first refund, `LoanChargePaidBy`
  records) wired into both the payment and refund walks' fee/penalty leaves, golden-verified.
- **Remaining Phase 1** (tracked): installment-fee `LoanInstallmentCharge` slices; the interest-
  recalculation branches of the ported handlers; re-age common/recalc strategy variants; the 19
  remaining accrual journal creators + the cash-basis processor; converting the handful of
  characterization/invariant tests to captured goldens where obtainable.

## Phase 2 — productionization (in progress)

- **Persistence (ADR-0026, DONE):** event-log recompute, not model-blob. `fc-loan-journal`
  persists an append-only per-loan operation log and replays it into the interest-schedule model;
  `replay == live` is byte-exact for the golden sequences, JSON round-trip is lossless and
  float-free. serde is confined to this crate.
- **Validation boundary (DONE):** `LoanJournal::validate()` turns the kernel's faithful silent
  clamps into explicit, typed, recordable rejections *before* replay, without touching the math.
- **Runnable host (proof-of-concept, DONE):** the `replay_cli` example runs the full
  validate → replay → project pipeline as a real process (exit 0 / 2 / 1); it is the shape the
  production gRPC adapter wraps.
- **Remaining Phase 2** (follow-on, need infra/scope beyond the domain):
  - **gRPC transport host + edge gateway** (ADR-0025/0011): tonic server exposing the schedule /
    allocation / replay surface, and the REST↔gRPC gateway. `fc-loan-journal` is the application
    surface it wraps.
  - **Observability** (ADR-0021): `tracing` spans + an audit record around every kernel call, at
    the transport layer (never in the hot math path).
  - **Automated dual-run parity gate** (plan/harness): run the Java EMI + processor suites and the
    Rust suites in CI and diff. Today the CI gates fmt/clippy/build/test/deny over the whole
    workspace (including the `replay == live` determinism golden); the cross-reference against a
    running Fineract needs JDK-21 + Postgres CI infra.

## How to run

```
cargo test --workspace --locked           # all gates' test step
cargo run -p fc-loan-journal --example replay_cli -- <journal.json>   # end-to-end host
```
