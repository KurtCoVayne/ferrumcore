//! Canonical external event schemas (ADR-0022). The `.avsc` files under `schemas/avro/` are copied **verbatim** from
//! the original `fineract-avro-schemas` and embedded here — they are the byte-compatible wire contract for external
//! consumers. We reproduce the Avro format via the `apache-avro` crate using these schemas as the sole authority.
//!
//! `MessageV1` is the event envelope; its `data` field carries the Avro-serialized payload as `bytes` (so payload
//! schemas evolve independently). `BulkMessagePayloadV1` aggregates `BulkMessageItemV1` records.

use apache_avro::Schema;

/// The event-envelope schema source (`MessageV1.avsc`), embedded verbatim.
pub const MESSAGE_V1_SCHEMA: &str = include_str!("../schemas/avro/MessageV1.avsc");
/// `BulkMessageItemV1.avsc`, embedded verbatim.
pub const BULK_MESSAGE_ITEM_V1_SCHEMA: &str =
    include_str!("../schemas/avro/BulkMessageItemV1.avsc");
/// `BulkMessagePayloadV1.avsc`, embedded verbatim (references `BulkMessageItemV1`).
pub const BULK_MESSAGE_PAYLOAD_V1_SCHEMA: &str =
    include_str!("../schemas/avro/BulkMessagePayloadV1.avsc");
/// The `BigDecimal` logical-type template (`bigdecimal.avsc`): `decimal`, precision 27, scale 8 — money parity.
pub const BIG_DECIMAL_SCHEMA: &str = include_str!("../schemas/avro/bigdecimal.avsc");

/// Parse the standalone `MessageV1` envelope schema.
///
/// # Errors
/// Returns the `apache-avro` parse error if the embedded schema is malformed (should never happen — it is verbatim).
pub fn message_v1() -> Result<Schema, apache_avro::Error> {
    Schema::parse_str(MESSAGE_V1_SCHEMA)
}

/// Parse the bulk schemas together so `BulkMessagePayloadV1`'s reference to `BulkMessageItemV1` resolves. Returns
/// `[BulkMessageItemV1, BulkMessagePayloadV1]` in the input order.
///
/// # Errors
/// Returns the `apache-avro` parse error if a schema is malformed or a cross-reference fails to resolve.
pub fn bulk_schemas() -> Result<Vec<Schema>, apache_avro::Error> {
    Schema::parse_list([BULK_MESSAGE_ITEM_V1_SCHEMA, BULK_MESSAGE_PAYLOAD_V1_SCHEMA])
}

/// Parse the `BigDecimal` logical-type template.
///
/// # Errors
/// Returns the `apache-avro` parse error if the embedded schema is malformed.
pub fn big_decimal() -> Result<Schema, apache_avro::Error> {
    Schema::parse_str(BIG_DECIMAL_SCHEMA)
}
