//! # fc-money
//!
//! FerrumCore's `Money` / decimal value type — the single source of all monetary arithmetic (ADR-0005).
//!
//! **Status: scaffold (no domain logic yet).** The implementation lands as the first Gate-1 domain task, built to the
//! spec proven by the B1 decimal-equivalence spike (`plan/spikes/decimal/FINDINGS.md`):
//!
//! - wrap the `bigdecimal` crate (value-equivalent to Java `BigDecimal` — 0 mismatches over 14,520 rows);
//! - carry an **explicit scale** re-applied after every op (Java scale-propagation);
//! - **manual integer round-to-scale** for division/quantize (not `bigdecimal`'s native `/`);
//! - format via a Java-`toPlainString` equivalent (no scientific notation; trailing zeros to scale; `-0` → `0`);
//! - validated against the real Fineract `Money` golden (`plan/harness/golden/money_corpus.tsv`, 3654/3654 match).
//!
//! No binary floating point is permitted anywhere in this crate (enforced by `clippy.toml` + workspace lints).
#![forbid(unsafe_code)]
