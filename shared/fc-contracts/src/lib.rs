//! # fc-contracts
//!
//! Shared cross-service contracts (Part-4 `shared/`): the typed API **error model** (faithful to the original
//! `ApiGlobalErrorResponse`/`ApiParameterError`) and the canonical **Avro event schemas** (ADR-0022), embedded
//! verbatim from `fineract-avro-schemas` as the byte-compatible external wire contract.
//!
//! This crate is IO/runtime/serde-free (Part-4 dependency invariant): it defines contract *shapes* and *schemas*;
//! JSON/Kafka transport lives in the integration adapters.
#![forbid(unsafe_code)]

pub mod avro;
pub mod error;

pub use error::{ApiErrorMessageArg, ApiGlobalErrorResponse, ApiParameterError};
