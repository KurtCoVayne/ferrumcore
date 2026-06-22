//! Conformance: every domain enum's `code` and Java name match the original (golden extracted from the source enums),
//! plus `from_code`/`from_java_name` round-trips. Guards the wire/DB contract.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use fc_domain_shared::{
    DaysInMonthType, DaysInYearType, InterestCalculationPeriodMethod, InterestMethod,
    PeriodFrequencyType, SavingsCompoundingInterestPeriodType, SavingsInterestCalculationType,
    SavingsPostingInterestPeriodType,
};

/// (code-by-java-name, java-name-by-code) for one enum, so the test can dispatch by enum name.
fn lookup(enum_name: &str, java: &str, code: i32) -> (Option<i32>, Option<String>) {
    macro_rules! probe {
        ($t:ty) => {{
            (
                <$t>::from_java_name(java).map(<$t>::code),
                <$t>::from_code(code).map(|v| v.java_name().to_string()),
            )
        }};
    }
    match enum_name {
        "DaysInYearType" => probe!(DaysInYearType),
        "DaysInMonthType" => probe!(DaysInMonthType),
        "PeriodFrequencyType" => probe!(PeriodFrequencyType),
        "InterestMethod" => probe!(InterestMethod),
        "InterestCalculationPeriodMethod" => probe!(InterestCalculationPeriodMethod),
        "SavingsInterestCalculationType" => probe!(SavingsInterestCalculationType),
        "SavingsCompoundingInterestPeriodType" => probe!(SavingsCompoundingInterestPeriodType),
        "SavingsPostingInterestPeriodType" => probe!(SavingsPostingInterestPeriodType),
        other => panic!("unknown enum {other}"),
    }
}

#[test]
fn enum_codes_match_original() {
    let corpus = include_str!("golden/enum_codes.tsv");
    let (mut total, mut bad) = (0u64, Vec::new());
    for line in corpus.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        let (enum_name, java, code) = (c[0], c[1], c[2].parse::<i32>().unwrap());
        let (code_by_name, name_by_code) = lookup(enum_name, java, code);
        total += 1;
        if code_by_name != Some(code) {
            bad.push(format!(
                "{enum_name}::{java} code: expected {code}, got {code_by_name:?}"
            ));
        }
        if name_by_code.as_deref() != Some(java) {
            bad.push(format!(
                "{enum_name} from_code({code}): expected {java}, got {name_by_code:?}"
            ));
        }
    }
    assert!(total >= 39, "golden too small: {total}");
    assert!(
        bad.is_empty(),
        "{} divergences:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
