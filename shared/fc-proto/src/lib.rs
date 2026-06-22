//! # fc-proto
//!
//! FerrumCore inter-service wire contracts (ADR-0025: synchronous gRPC, tonic/prost). The canonical source of truth is
//! the `.proto` IDL under `proto/`; the Rust types here are **generated** from it at build time by the pure-Rust
//! `protox` compiler (no system `protoc` — reproducible builds, ADR-0002). This crate is the *only* place cross-service
//! message shapes are defined; services depend on it and never share domain/persistence crates (Part-4 invariant).
//!
//! Money crosses the wire as an exact decimal **string** (fc-money `to_plain_string`) — never binary float (ADR-0005).
#![forbid(unsafe_code)]
// Generated prost code does not meet our hand-written lint bar; scope-allow on the generated module only.
#![allow(clippy::all, clippy::pedantic, clippy::nursery)]

/// Generated protobuf types, namespaced by their proto package `ferrumcore.core_ledger.v1`.
pub mod ferrumcore {
    pub mod core_ledger {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/ferrumcore.core_ledger.v1.rs"));
        }
    }
}

/// Convenience re-export of the core-ledger v1 contracts.
pub use ferrumcore::core_ledger::v1;
