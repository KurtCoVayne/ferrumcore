//! Error-model parity: status codes, globalisation message codes, and nested-error shapes match the original
//! `ApiGlobalErrorResponse` factories exactly (the API error contract seen by existing consumers).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test code

use fc_contracts::{ApiGlobalErrorResponse, ApiParameterError};

#[test]
fn status_codes_and_codes_match_original() {
    let e = ApiGlobalErrorResponse::un_authenticated();
    assert_eq!(e.http_status_code, 401);
    assert_eq!(
        e.user_message_globalisation_code,
        "error.msg.not.authenticated"
    );

    assert_eq!(
        ApiGlobalErrorResponse::invalid_tenant_identifier().http_status_code,
        401
    );
    assert_eq!(
        ApiGlobalErrorResponse::un_authorized("nope").http_status_code,
        403
    );
    assert_eq!(
        ApiGlobalErrorResponse::not_found("x", "y", &[]).http_status_code,
        404
    );
    assert_eq!(
        ApiGlobalErrorResponse::bad_client_request("x", "y", vec![]).http_status_code,
        400
    );
    assert_eq!(
        ApiGlobalErrorResponse::not_implemented("x", "y", &[]).http_status_code,
        501
    );
    assert_eq!(
        ApiGlobalErrorResponse::server_side_error("x", "y", &[]).http_status_code,
        500
    );
    assert_eq!(
        ApiGlobalErrorResponse::service_unavailable("x", "y", &[]).http_status_code,
        503
    );
    assert_eq!(
        ApiGlobalErrorResponse::invalid_instance_type_method("POST").http_status_code,
        405
    );
    assert_eq!(
        ApiGlobalErrorResponse::loan_is_locked(7).http_status_code,
        409
    );
}

#[test]
fn not_found_wraps_resource_identifier_error() {
    let e = ApiGlobalErrorResponse::not_found("error.msg.loan.id.invalid", "Loan 5 not found", &[]);
    assert_eq!(
        e.user_message_globalisation_code,
        "error.msg.resource.not.found"
    );
    assert_eq!(
        e.default_user_message,
        "The requested resource is not available."
    );
    assert_eq!(e.errors.len(), 1);
    assert_eq!(
        e.errors[0].user_message_globalisation_code,
        "error.msg.loan.id.invalid"
    );
    assert_eq!(e.errors[0].default_user_message, "Loan 5 not found");
}

#[test]
fn conflict_detail_string_assembly_is_faithful() {
    // type + identifier -> "... on loan [5], ..." and code "error.msg.platform.service.loan.conflict".
    let e = ApiGlobalErrorResponse::conflict(Some("loan"), Some("5"));
    assert_eq!(e.http_status_code, 409);
    assert_eq!(
        e.user_message_globalisation_code,
        "error.msg.platform.service.loan.conflict"
    );
    assert!(
        e.default_user_message.contains(" on loan [5]"),
        "{}",
        e.default_user_message
    );

    // null type -> "unknown", no " on ..." segment.
    let u = ApiGlobalErrorResponse::conflict(None, None);
    assert_eq!(
        u.user_message_globalisation_code,
        "error.msg.platform.service.unknown.conflict"
    );
    assert!(
        !u.default_user_message.contains(" on "),
        "{}",
        u.default_user_message
    );
}

#[test]
fn parameter_error_with_value_carries_parameter_and_value() {
    let pe = ApiParameterError::parameter_error_with_value(
        "validation.msg.loan.principal.not.greater.than.zero",
        "Principal must be > 0",
        "principal",
        "-1",
        &["0".to_string()],
    );
    assert_eq!(pe.parameter_name.as_deref(), Some("principal"));
    assert_eq!(pe.value.as_deref(), Some("-1"));
    assert_eq!(pe.args.len(), 1);
    assert_eq!(pe.args[0].value.as_deref(), Some("0"));
}

#[test]
fn domain_rule_violation_shape() {
    let e = ApiGlobalErrorResponse::domain_rule_violation(
        "error.msg.loan.overpaid",
        "Loan overpaid",
        &[],
    );
    assert_eq!(e.http_status_code, 403);
    assert_eq!(
        e.user_message_globalisation_code,
        "validation.msg.domain.rule.violation"
    );
    assert_eq!(e.errors.len(), 1);
    assert_eq!(
        e.errors[0].user_message_globalisation_code,
        "error.msg.loan.overpaid"
    );
}
