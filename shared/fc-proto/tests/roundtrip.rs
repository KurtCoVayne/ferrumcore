//! Wire round-trip: every contract message encodes to protobuf bytes and decodes back identically; enum discriminants
//! equal the original Fineract codes (the wire/DB contract). Proves the generated prost types are usable and stable.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use fc_proto::v1;
use prost::Message;

fn money(amount: &str) -> v1::Money {
    v1::Money {
        amount: amount.to_string(),
        currency_code: "USD".to_string(),
        scale: 2,
    }
}

#[test]
fn schedule_request_roundtrips() {
    let req = v1::GenerateScheduleRequest {
        disbursed: Some(money("1000.00")),
        annual_nominal_interest_rate: "12.00".to_string(),
        number_of_repayments: 3,
        repay_every: 1,
        repayment_frequency: v1::PeriodFrequencyType::Months as i32,
        interest_method: v1::InterestMethod::DecliningBalance as i32,
        interest_calculation_period: v1::InterestCalculationPeriodMethod::Daily as i32,
        grace_on_principal_periods: 0,
        days_in_year: 365,
        days_in_month_30: false,
        disbursement_date: Some(v1::LocalDate { epoch_day: 19723 }), // 2024-01-01
    };
    let bytes = req.encode_to_vec();
    let back = v1::GenerateScheduleRequest::decode(bytes.as_slice()).unwrap();
    assert_eq!(req, back);
}

#[test]
fn schedule_response_roundtrips() {
    let resp = v1::GenerateScheduleResponse {
        schedule: Some(v1::LoanSchedule {
            installments: vec![
                v1::LoanScheduleInstallment {
                    period: 1,
                    from_date: Some(v1::LocalDate { epoch_day: 19723 }),
                    due_date: Some(v1::LocalDate { epoch_day: 19754 }),
                    emi: Some(money("340.02")),
                    principal: Some(money("330.02")),
                    interest: Some(money("10.00")),
                    outstanding_principal: Some(money("669.98")),
                },
                v1::LoanScheduleInstallment {
                    period: 2,
                    from_date: Some(v1::LocalDate { epoch_day: 19754 }),
                    due_date: Some(v1::LocalDate { epoch_day: 19783 }),
                    emi: Some(money("340.02")),
                    principal: Some(money("333.32")),
                    interest: Some(money("6.70")),
                    outstanding_principal: Some(money("336.66")),
                },
            ],
        }),
    };
    let bytes = resp.encode_to_vec();
    let back = v1::GenerateScheduleResponse::decode(bytes.as_slice()).unwrap();
    assert_eq!(resp, back);
    assert_eq!(back.schedule.unwrap().installments.len(), 2);
}

#[test]
fn enum_discriminants_match_original_codes() {
    // LoanStatus — sparse codes are the DB/wire contract.
    assert_eq!(v1::LoanStatus::Invalid as i32, 0);
    assert_eq!(v1::LoanStatus::SubmittedAndPendingApproval as i32, 100);
    assert_eq!(v1::LoanStatus::Approved as i32, 200);
    assert_eq!(v1::LoanStatus::Active as i32, 300);
    assert_eq!(v1::LoanStatus::TransferInProgress as i32, 303);
    assert_eq!(v1::LoanStatus::TransferOnHold as i32, 304);
    assert_eq!(v1::LoanStatus::WithdrawnByClient as i32, 400);
    assert_eq!(v1::LoanStatus::Rejected as i32, 500);
    assert_eq!(v1::LoanStatus::ClosedObligationsMet as i32, 600);
    assert_eq!(v1::LoanStatus::ClosedWrittenOff as i32, 601);
    assert_eq!(
        v1::LoanStatus::ClosedRescheduleOutstandingAmount as i32,
        602
    );
    assert_eq!(v1::LoanStatus::Overpaid as i32, 700);
    // Other contract enums.
    assert_eq!(v1::InterestMethod::DecliningBalance as i32, 0);
    assert_eq!(v1::InterestMethod::Flat as i32, 1);
    assert_eq!(v1::InterestCalculationPeriodMethod::Daily as i32, 0);
    assert_eq!(
        v1::InterestCalculationPeriodMethod::SameAsRepayment as i32,
        1
    );
    assert_eq!(v1::PeriodFrequencyType::Months as i32, 2);
}

#[test]
fn signed_epoch_day_roundtrips_negative() {
    // sint64 zig-zag encoding must preserve pre-epoch dates.
    let d = v1::LocalDate { epoch_day: -10_957 }; // ~1940
    let bytes = d.encode_to_vec();
    assert_eq!(
        v1::LocalDate::decode(bytes.as_slice()).unwrap().epoch_day,
        -10_957
    );
}
