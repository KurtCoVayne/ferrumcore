//! # fc-domain-shared
//!
//! Cross-cutting domain value objects + enumerations shared across services. Enum **integer codes and Java constant
//! names are faithful to the original** (they are part of the wire/DB contract) — verified by the conformance golden
//! in `tests/`. Pure: no IO, no framework. Sources: `portfolio/common/domain`, `portfolio/loanproduct/domain`,
//! `portfolio/savings`, `infrastructure/core/domain/ExternalId` (`/Users/alejandro/fineract`).
#![forbid(unsafe_code)]

/// Declares an enum whose variants carry the original's integer `code` and Java constant `name`.
macro_rules! coded_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $code:expr => $java:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            /// Numeric code, identical to the original enum.
            #[must_use] pub fn code(self) -> i32 { match self { $(Self::$variant => $code),+ } }
            /// Resolve from the numeric code.
            #[must_use] pub fn from_code(c: i32) -> Option<Self> {
                match c { $(c if c == $code => Some(Self::$variant),)+ _ => None }
            }
            /// The original Java enum-constant name (e.g. `DAYS_360`, `QUATERLY`).
            #[must_use] pub fn java_name(self) -> &'static str { match self { $(Self::$variant => $java),+ } }
            /// Resolve from the original Java enum-constant name.
            #[must_use] pub fn from_java_name(s: &str) -> Option<Self> {
                match s { $($java => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

coded_enum!(
    /// Day-count basis for the year (`DaysInYearType`).
    DaysInYearType { Invalid = 0 => "INVALID", Actual = 1 => "ACTUAL",
        Days360 = 360 => "DAYS_360", Days364 = 364 => "DAYS_364", Days365 = 365 => "DAYS_365" }
);
coded_enum!(
    /// Day-count basis for the month (`DaysInMonthType`).
    DaysInMonthType { Invalid = 0 => "INVALID", Actual = 1 => "ACTUAL", Days30 = 30 => "DAYS_30" }
);
coded_enum!(
    /// Repayment/period frequency (`PeriodFrequencyType`).
    PeriodFrequencyType { Days = 0 => "DAYS", Weeks = 1 => "WEEKS", Months = 2 => "MONTHS",
        Years = 3 => "YEARS", WholeTerm = 4 => "WHOLE_TERM", Invalid = 5 => "INVALID" }
);
coded_enum!(
    /// Loan interest method (`InterestMethod`).
    InterestMethod { DecliningBalance = 0 => "DECLINING_BALANCE", Flat = 1 => "FLAT", Invalid = 2 => "INVALID" }
);
coded_enum!(
    /// Interest calculation period (`InterestCalculationPeriodMethod`).
    InterestCalculationPeriodMethod { Daily = 0 => "DAILY",
        SameAsRepaymentPeriod = 1 => "SAME_AS_REPAYMENT_PERIOD", Invalid = 2 => "INVALID" }
);
coded_enum!(
    /// Savings interest calculation type (`SavingsInterestCalculationType`). Both balance modes implemented (verified).
    SavingsInterestCalculationType { Invalid = 0 => "INVALID", DailyBalance = 1 => "DAILY_BALANCE",
        AverageDailyBalance = 2 => "AVERAGE_DAILY_BALANCE" }
);
coded_enum!(
    /// Savings compounding period (`SavingsCompoundingInterestPeriodType`). NB: original spells code 5 `QUATERLY`.
    SavingsCompoundingInterestPeriodType { Invalid = 0 => "INVALID", Daily = 1 => "DAILY", Monthly = 4 => "MONTHLY",
        Quarterly = 5 => "QUATERLY", BiAnnual = 6 => "BI_ANNUAL", Annual = 7 => "ANNUAL" }
);
coded_enum!(
    /// Savings posting period (`SavingsPostingInterestPeriodType`). NB: code 5 `QUATERLY`, code 6 `BIANNUAL`.
    SavingsPostingInterestPeriodType { Invalid = 0 => "INVALID", Daily = 1 => "DAILY", Monthly = 4 => "MONTHLY",
        Quarterly = 5 => "QUATERLY", BiAnnual = 6 => "BIANNUAL", Annual = 7 => "ANNUAL",
        AnniversaryMonthly = 8 => "ANNIVERSARY_MONTHLY", AnniversaryQuarterly = 9 => "ANNIVERSARY_QUARTERLY",
        AnniversaryBiAnnual = 10 => "ANNIVERSARY_BIANNUAL", AnniversaryAnnual = 11 => "ANNIVERSARY_ANNUAL" }
);

/// Optional external identifier (`ExternalId`). Blank values are rejected; absence is [`ExternalId::empty`].
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExternalId(Option<String>);

/// Error constructing an [`ExternalId`] from a blank value (mirrors the original's validation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlankExternalId;

impl ExternalId {
    /// Construct from a non-blank value (the original throws on blank).
    ///
    /// # Errors
    /// Returns [`BlankExternalId`] when `value` is empty or whitespace.
    pub fn try_new(value: impl Into<String>) -> Result<Self, BlankExternalId> {
        let v = value.into();
        if v.trim().is_empty() {
            Err(BlankExternalId)
        } else {
            Ok(Self(Some(v)))
        }
    }
    /// The empty (absent) external id.
    #[must_use]
    pub fn empty() -> Self {
        Self(None)
    }
    /// Whether this id is absent.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }
    /// The underlying value, if present.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}
