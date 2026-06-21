# FerrumCore

A ground-up reimplementation of the **Apache Fineract** core banking platform, built to be
**provably behaviorally equivalent** to the original system before any redesign or optimization
is attempted.

> Mission-critical financial system. Correctness, safety, auditability, deterministic behavior,
> and business-rule preservation rank **above** development speed and runtime performance.

## Prime directive

The new system must exhibit behavior **identical** to the original Fineract implementation
(`/Users/alejandro/fineract`, branch `develop`, HEAD `5d3de9cf0`). No feature is "done" until its
behavior is validated against the original; **every original test must pass unchanged** against
FerrumCore, and any undocumented behavior discovered during implementation must yield new tests.

## Guiding principles (non-negotiable)

1. Correctness before performance.
2. Safety before convenience.
3. Explicitness before abstraction.
4. Verification before optimization.
5. Business-rule preservation before architectural improvement.
6. No feature is complete unless validated against the original.
7. Every original test must pass against the new implementation.
8. Missing behavioral coverage discovered → new tests.
9. The implementation must be able to demonstrate behavioral equivalence.
10. Performance work is deferred until full behavioral parity is achieved.

## Where the plan lives

The complete implementation blueprint and the proposed source skeleton live under **`plan/`**,
which is **gitignored** — it is pre-implementation working material, not committed source.

```
plan/
  00-MASTER.md                              index + cross-map + traceability
  phase-0-specification-extraction.md       invariants & guarantees (the spec)
  phase-1-verification-foundation.md        test strategy before code
  phase-2-domain-modeling.md                aggregates/VOs/commands/events + order
  phase-3-architecture-skeleton.md          layered project structure
  phase-4-subsystem-implementation-order.md dependency-aware sequence + gates
  phase-5-security-first-foundations.md     authn/authz/secrets/crypto/audit
  phase-6-data-integrity-strategy.md        tx/idempotency/concurrency/recovery
  phase-7-behavioral-parity-program.md      per-feature parity ledger
  phase-8-readiness-gates.md                7 mandatory gates, measurable criteria
  phase-9-risk-register.md                  ranked risks across 5 categories
  final-roadmap.md                          week-by-week roadmap
  structure/                                proposed implementation skeleton (dir tree)
```

## Source-of-truth references

The blueprint is grounded in the prior evidence-based analysis of the original system, kept at
`/Users/alejandro/fineract/modernization/` (architecture review, subsystem decomposition,
contracts, test inventory, evidence index). Every recommendation here is traceable to concrete
files, classes, and tests in the original repository.

## Status

Pre-implementation. This repository currently contains only the planning blueprint (gitignored)
and this shell. Production code lands only after **Gate 1 (Specification completeness)** passes —
see `plan/phase-8-readiness-gates.md`.
