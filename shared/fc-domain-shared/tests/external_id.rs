//! Unit coverage for the `ExternalId` value object (blank-rejection mirrors the original).
#![allow(clippy::unwrap_used)] // test code

use fc_domain_shared::{
    ExternalId, SavingsCompoundingInterestPeriodType, SavingsPostingInterestPeriodType,
};

#[test]
fn external_id_behaviour() {
    let id = ExternalId::try_new("ext-123").unwrap();
    assert!(!id.is_empty());
    assert_eq!(id.value(), Some("ext-123"));

    let empty = ExternalId::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.value(), None);
    assert_eq!(empty, ExternalId::default());

    // blank / whitespace rejected (original throws on blank)
    assert!(ExternalId::try_new("").is_err());
    assert!(ExternalId::try_new("   ").is_err());
}

#[test]
fn savings_quarterly_misspelling_preserved() {
    // the original spells code 5 "QUATERLY" (sic) in both savings enums — part of the contract
    assert_eq!(
        SavingsCompoundingInterestPeriodType::Quarterly.java_name(),
        "QUATERLY"
    );
    assert_eq!(SavingsCompoundingInterestPeriodType::Quarterly.code(), 5);
    assert_eq!(
        SavingsPostingInterestPeriodType::BiAnnual.java_name(),
        "BIANNUAL"
    );
    assert_eq!(
        SavingsPostingInterestPeriodType::AnniversaryAnnual.code(),
        11
    );
}
