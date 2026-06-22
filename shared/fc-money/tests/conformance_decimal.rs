//! Conformance: replay the Java-`BigDecimal` golden corpus (captured in B1/B2) through fc-money's decimal core and
//! assert EXACT `toPlainString` equality. This is the permanent parity gate for the decimal foundation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_money::decimal;
use std::str::FromStr;

fn mode(name: &str) -> RoundingMode {
    match name {
        "HALF_UP" => RoundingMode::HalfUp,
        "HALF_DOWN" => RoundingMode::HalfDown,
        "HALF_EVEN" => RoundingMode::HalfEven,
        "UP" => RoundingMode::Up,
        "DOWN" => RoundingMode::Down,
        "FLOOR" => RoundingMode::Floor,
        "CEILING" => RoundingMode::Ceiling,
        other => panic!("unknown mode {other}"),
    }
}
fn bd(s: &str) -> BigDecimal {
    BigDecimal::from_str(s).unwrap()
}

#[test]
fn decimal_matches_java_bigdecimal_golden() {
    let corpus = include_str!("golden/decimal_corpus.tsv");
    let (mut total, mut mismatches) = (0u64, Vec::new());
    for line in corpus.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let (op, a, b, sc, m, java) = (c[1], c[2], c[3], c[4], c[5], c[6]);
        let got = match op {
            "ADD" => decimal::to_plain_string(&decimal::add(&bd(a), &bd(b))),
            "SUB" => decimal::to_plain_string(&decimal::sub(&bd(a), &bd(b))),
            "MUL" => decimal::to_plain_string(&decimal::mul(&bd(a), &bd(b))),
            "CMP" => decimal::compare(&bd(a), &bd(b)).to_string(),
            "SETSCALE" => {
                decimal::to_plain_string(&decimal::set_scale(&bd(a), sc.parse().unwrap(), mode(m)))
            }
            "DIVSCALE" => decimal::to_plain_string(&decimal::div(
                &bd(a),
                &bd(b),
                sc.parse().unwrap(),
                mode(m),
            )),
            _ => continue,
        };
        total += 1;
        if got != java && mismatches.len() < 20 {
            mismatches.push(format!(
                "{op} a={a} b={b} s={sc} {m}: java='{java}' got='{got}'"
            ));
        } else if got != java {
            mismatches.push(String::new());
        }
    }
    assert!(total > 14_000, "corpus too small: {total}");
    assert!(
        mismatches.is_empty(),
        "{} / {total} rows diverge from Java BigDecimal:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
