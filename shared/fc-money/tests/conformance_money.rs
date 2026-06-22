//! Conformance: replay the REAL Fineract `Money` golden corpus (captured in B7) through fc-money's `Money` and assert
//! EXACT equality. Permanent parity gate for currency-scale normalization + rounding.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use bigdecimal::BigDecimal;
use bigdecimal::RoundingMode;
use fc_money::Money;
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
fn money_matches_real_fineract_money_golden() {
    let corpus = include_str!("golden/money_corpus.tsv");
    let (mut total, mut mismatches) = (0u64, Vec::new());
    for line in corpus.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let (op, d, a, b, m, java) = (c[1], c[2].parse::<i64>().unwrap(), c[3], c[4], c[5], c[6]);
        let rm = mode(m);
        let ma = Money::of(d, rm, &bd(a));
        let got = match op {
            "NORMALIZE" => ma.to_plain_string(),
            "PLUS" => ma.plus(&Money::of(d, rm, &bd(b))).to_plain_string(),
            "MINUS" => ma.minus(&Money::of(d, rm, &bd(b))).to_plain_string(),
            "MUL" => ma.multiplied_by(&bd(b)).to_plain_string(),
            "DIV" => ma.divided_by(&bd(b)).to_plain_string(),
            _ => continue,
        };
        total += 1;
        if got != java {
            mismatches.push(format!(
                "{op} d={d} a={a} b={b} {m}: java='{java}' got='{got}'"
            ));
        }
    }
    assert!(total > 3_000, "corpus too small: {total}");
    assert!(
        mismatches.is_empty(),
        "{} / {total} rows diverge from real Fineract Money:\n{}",
        mismatches.len(),
        mismatches
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
