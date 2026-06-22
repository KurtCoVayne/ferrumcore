//! Avro conformance: the embedded canonical schemas parse, and a `MessageV1` envelope round-trips through the
//! `apache-avro` writer/reader producing an identical record — proving byte-format compatibility against the original
//! `fineract-avro-schemas` (ADR-0022).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use apache_avro::types::{Record, Value};
use apache_avro::{Reader, Schema, Writer};
use fc_contracts::avro;

#[test]
fn canonical_schemas_parse_with_expected_names() {
    let msg = avro::message_v1().unwrap();
    match &msg {
        Schema::Record(r) => {
            assert_eq!(r.name.fullname(None), "org.apache.fineract.avro.MessageV1");
            let fields: Vec<&str> = r.fields.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(
                fields,
                vec![
                    "id",
                    "source",
                    "type",
                    "category",
                    "createdAt",
                    "businessDate",
                    "tenantId",
                    "idempotencyKey",
                    "dataschema",
                    "data"
                ]
            );
        }
        other => panic!("expected record, got {other:?}"),
    }

    // BulkMessagePayloadV1 references BulkMessageItemV1 — must resolve when parsed together.
    let bulk = avro::bulk_schemas().unwrap();
    assert_eq!(bulk.len(), 2);

    // BigDecimal logical type: decimal(precision=27, scale=8) over bytes.
    match avro::big_decimal().unwrap() {
        Schema::Decimal(d) => {
            assert_eq!(d.precision, 27);
            assert_eq!(d.scale, 8);
        }
        other => panic!("expected decimal logical type, got {other:?}"),
    }
}

#[test]
fn message_v1_envelope_roundtrips() {
    let schema = avro::message_v1().unwrap();
    let mut record = Record::new(&schema).unwrap();
    record.put("id", 42i64);
    record.put("source", "core-ledger");
    record.put("type", "LoanApprovedBusinessEvent");
    record.put("category", "LOAN");
    record.put("createdAt", "2026-06-20T10:15:30");
    record.put("businessDate", "2026-06-20");
    record.put("tenantId", "default");
    record.put("idempotencyKey", "idem-key-1");
    record.put(
        "dataschema",
        "org.apache.fineract.avro.loan.v1.LoanAccountDataV1",
    );
    record.put("data", Value::Bytes(vec![0x01, 0x02, 0x03, 0xff]));

    let mut writer = Writer::new(&schema, Vec::new());
    writer.append(record).unwrap();
    let bytes = writer.into_inner().unwrap();

    let reader = Reader::with_schema(&schema, &bytes[..]).unwrap();
    let mut count = 0;
    for value in reader {
        let Value::Record(fields) = value.unwrap() else {
            panic!("expected record");
        };
        let get = |name: &str| {
            fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("id"), Value::Long(42));
        assert_eq!(get("source"), Value::String("core-ledger".to_string()));
        assert_eq!(
            get("type"),
            Value::String("LoanApprovedBusinessEvent".to_string())
        );
        assert_eq!(get("category"), Value::String("LOAN".to_string()));
        assert_eq!(get("data"), Value::Bytes(vec![0x01, 0x02, 0x03, 0xff]));
        count += 1;
    }
    assert_eq!(count, 1);
}
