//! # fc-money
//!
//! FerrumCore's exact-decimal `Money` value type — the single source of all monetary arithmetic (ADR-0005),
//! built to be **bit-for-bit value-equivalent to Java `BigDecimal` + `MathContext`**.
//!
//! Built to the spec proven by the B1 decimal-equivalence spike (`plan/spikes/decimal/FINDINGS.md`):
//! - wraps the `bigdecimal` crate (0 value mismatches over 14,520 ops vs Java);
//! - explicit scale, re-applied after every operation (Java scale-propagation);
//! - manual integer round-to-scale for division/quantize (not `bigdecimal`'s native `/`);
//! - Java-`toPlainString` formatting (no scientific notation; trailing zeros to scale; `-0` → `0`).
//!
//! Continuously validated against the original by the committed conformance suites in `tests/` (the golden corpora
//! captured from Java `BigDecimal` and the real Fineract `Money`). No binary floating point anywhere (enforced by lints).
#![forbid(unsafe_code)]

pub mod decimal;
pub mod money;

pub use money::Money;
