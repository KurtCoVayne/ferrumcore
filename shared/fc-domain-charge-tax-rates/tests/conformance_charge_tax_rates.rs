//! Conformance for fc-domain-charge-tax-rates:
//!  * charge enum codes/names round-trip against the original (golden/charge_enum_codes.tsv);
//!  * `split_tax` reproduces `TaxUtils.splitTax`'s double-precision result EXACTLY for every captured case
//!    (golden/tax_split.tsv, produced by the real JDK-21 expression).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::{BigDecimal, RoundingMode};
use fc_domain_charge_tax_rates::{
    ChargeAppliesTo, ChargeCalculationType, ChargePaymentMode, ChargeTimeType, TaxComponent,
    split_tax,
};
use fc_money::decimal::to_plain_string;
use std::str::FromStr;
use time::{Date, Month};

// Each charge enum: (java_name -> code, code -> java_name) round-trip, dispatched by enum name.
fn lookup(enum_name: &str, java: &str) -> (Option<i32>, Option<String>) {
    macro_rules! probe {
        ($t:ty) => {{
            let by_name = <$t>::from_java_name(java).map(|v| v.code());
            let by_code = by_name
                .and_then(<$t>::from_code)
                .map(|v| v.java_name().to_string());
            (by_name, by_code)
        }};
    }
    match enum_name {
        "ChargeTimeType" => probe!(ChargeTimeType),
        "ChargeCalculationType" => probe!(ChargeCalculationType),
        "ChargeAppliesTo" => probe!(ChargeAppliesTo),
        "ChargePaymentMode" => probe!(ChargePaymentMode),
        other => panic!("unknown enum {other}"),
    }
}

#[test]
fn charge_enum_codes_match_original() {
    let mut count = 0u32;
    for line in include_str!("golden/charge_enum_codes.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let (enum_name, java, code) = (c[0], c[1], c[2].parse::<i32>().unwrap());
        let (got_code, got_name) = lookup(enum_name, java);
        assert_eq!(got_code, Some(code), "{enum_name}::{java} code");
        assert_eq!(
            got_name.as_deref(),
            Some(java),
            "{enum_name} code {code} -> name"
        );
        count += 1;
    }
    assert!(count >= 29, "unexpected enum-code golden size: {count}");
}

#[test]
fn split_tax_matches_real_double_expression() {
    let any = Date::from_calendar_date(2024, Month::June, 15).unwrap();
    let mut mismatches = Vec::new();
    let mut count = 0u32;
    for line in include_str!("golden/tax_split.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let base = BigDecimal::from_str(c[0]).unwrap();
        let comp = TaxComponent {
            percentage: Some(BigDecimal::from_str(c[1]).unwrap()),
            start: None,
            end: None,
        };
        let scale: i64 = c[2].parse().unwrap();
        let expected = c[3];
        let split = split_tax(
            &base,
            std::slice::from_ref(&comp),
            any,
            scale,
            RoundingMode::HalfEven,
        );
        assert_eq!(split.len(), 1, "row {c:?}");
        let got = to_plain_string(&split[0].1);
        if got != expected {
            mismatches.push(format!(
                "base={} pct={} scale={}: golden={expected} got={got}",
                c[0], c[1], c[2]
            ));
        }
        count += 1;
    }
    assert!(count >= 20, "unexpected tax golden size: {count}");
    assert!(
        mismatches.is_empty(),
        "tax split diverged from TaxUtils:\n{}",
        mismatches.join("\n")
    );
}
