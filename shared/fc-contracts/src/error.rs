//! Typed API error model — faithful port of the original `ApiGlobalErrorResponse` / `ApiParameterError` /
//! `ApiErrorMessageArg` (`fineract-core/.../infrastructure/core/data`). This is the canonical cross-service error
//! contract; JSON/HTTP serialization is a transport-adapter concern (this crate stays serde/IO-free per Part-4).
//!
//! Status codes and message-globalisation codes are reproduced exactly so existing API consumers see identical errors.

/// HTTP status codes used by the original error factories (jakarta `HttpServletResponse.SC_*`).
mod status {
    pub const BAD_REQUEST: u16 = 400;
    pub const UNAUTHORIZED: u16 = 401;
    pub const FORBIDDEN: u16 = 403;
    pub const NOT_FOUND: u16 = 404;
    pub const METHOD_NOT_ALLOWED: u16 = 405;
    pub const CONFLICT: u16 = 409;
    pub const INTERNAL_SERVER_ERROR: u16 = 500;
    pub const NOT_IMPLEMENTED: u16 = 501;
    pub const SERVICE_UNAVAILABLE: u16 = 503;
}

/// `ApiErrorMessageArg` — a positional argument for message interpolation. The original holds an `Object`; over the
/// wire it is stringified, so the contract carries its `value` as text.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApiErrorMessageArg {
    pub value: Option<String>,
}

impl ApiErrorMessageArg {
    /// `ApiErrorMessageArg.from(object)`.
    #[must_use]
    pub fn from<S: Into<String>>(value: S) -> Self {
        Self {
            value: Some(value.into()),
        }
    }

    fn args(values: &[String]) -> Vec<Self> {
        values.iter().map(|v| Self::from(v.clone())).collect()
    }
}

/// `ApiParameterError` — one error entry (a field/data-validation error or a general error).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApiParameterError {
    pub user_message_globalisation_code: String,
    pub developer_message: String,
    pub default_user_message: String,
    pub parameter_name: Option<String>,
    pub value: Option<String>,
    pub args: Vec<ApiErrorMessageArg>,
}

impl ApiParameterError {
    /// `ApiParameterError.generalError(code, defaultUserMessage, args...)`.
    #[must_use]
    pub fn general_error(code: &str, default_user_message: &str, args: &[String]) -> Self {
        Self {
            user_message_globalisation_code: code.to_string(),
            developer_message: default_user_message.to_string(),
            default_user_message: default_user_message.to_string(),
            parameter_name: None,
            value: None,
            args: ApiErrorMessageArg::args(args),
        }
    }

    /// `ApiParameterError.resourceIdentifierNotFound(code, defaultUserMessage, args...)`.
    #[must_use]
    pub fn resource_identifier_not_found(
        code: &str,
        default_user_message: &str,
        args: &[String],
    ) -> Self {
        Self::general_error(code, default_user_message, args)
    }

    /// `ApiParameterError.parameterError(code, developerMessage, defaultUserMessage, parameterName, args...)`.
    #[must_use]
    pub fn parameter_error(
        code: &str,
        developer_message: &str,
        default_user_message: &str,
        parameter_name: &str,
        args: &[String],
    ) -> Self {
        Self {
            user_message_globalisation_code: code.to_string(),
            developer_message: developer_message.to_string(),
            default_user_message: default_user_message.to_string(),
            parameter_name: Some(parameter_name.to_string()),
            value: None,
            args: ApiErrorMessageArg::args(args),
        }
    }

    /// `ApiParameterError.parameterErrorWithValue(code, defaultUserMessage, parameterName, value, args...)`.
    #[must_use]
    pub fn parameter_error_with_value(
        code: &str,
        default_user_message: &str,
        parameter_name: &str,
        value: &str,
        args: &[String],
    ) -> Self {
        Self {
            user_message_globalisation_code: code.to_string(),
            developer_message: default_user_message.to_string(),
            default_user_message: default_user_message.to_string(),
            parameter_name: Some(parameter_name.to_string()),
            value: Some(value.to_string()),
            args: ApiErrorMessageArg::args(args),
        }
    }
}

/// `ApiGlobalErrorResponse` — the top-level error response (HTTP status + aggregated `ApiParameterError`s).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ApiGlobalErrorResponse {
    pub http_status_code: u16,
    pub user_message_globalisation_code: String,
    pub developer_message: String,
    pub default_user_message: String,
    pub errors: Vec<ApiParameterError>,
}

impl ApiGlobalErrorResponse {
    /// `ApiGlobalErrorResponse.create(statusCode, msgCode, developerMessage, defaultUserMessage, errors)`.
    #[must_use]
    pub fn create(
        status_code: u16,
        msg_code: &str,
        developer_message: &str,
        default_user_message: &str,
        errors: Vec<ApiParameterError>,
    ) -> Self {
        Self {
            http_status_code: status_code,
            user_message_globalisation_code: msg_code.to_string(),
            developer_message: developer_message.to_string(),
            default_user_message: default_user_message.to_string(),
            errors,
        }
    }

    /// `unAuthenticated()` — 401.
    #[must_use]
    pub fn un_authenticated() -> Self {
        Self::create(
            status::UNAUTHORIZED,
            "error.msg.not.authenticated",
            "Invalid authentication details were passed in api request.",
            "Unauthenticated. Please login.",
            Vec::new(),
        )
    }

    /// `invalidTenantIdentifier()` — 401.
    #[must_use]
    pub fn invalid_tenant_identifier() -> Self {
        Self::create(
            status::UNAUTHORIZED,
            "error.msg.invalid.tenant.identifier",
            "Invalid tenant details were passed in api request.",
            "Invalid tenant identifier provided with request.",
            Vec::new(),
        )
    }

    /// `invalidInstanceTypeMethod(method)` — 405.
    #[must_use]
    pub fn invalid_instance_type_method(method: &str) -> Self {
        Self::create(
            status::METHOD_NOT_ALLOWED,
            "error.msg.invalid.instance.type",
            &format!("Invalid instance type called in api request for the method {method}"),
            &format!("Invalid method {method} used with request to this instance type."),
            Vec::new(),
        )
    }

    /// `loanIsLocked(loanId)` — 409.
    #[must_use]
    pub fn loan_is_locked(loan_id: i64) -> Self {
        let msg = format!("Loan is locked by the COB job. Loan ID: {loan_id}");
        Self::create(
            status::CONFLICT,
            "error.msg.loan.locked",
            &msg,
            &msg,
            Vec::new(),
        )
    }

    /// `conflict(type, identifier)` — 409 (faithful detail-string assembly).
    #[must_use]
    pub fn conflict(type_: Option<&str>, identifier: Option<&str>) -> Self {
        let type_ = type_.unwrap_or("unknown");
        let mut details = if type_ == "unknown" {
            String::new()
        } else {
            format!(" on {type_}")
        };
        if let Some(id) = identifier {
            details.push_str(&format!(" [{id}]"));
        }
        let msg = format!(
            "The server is currently unable to handle the request due to concurrent modification {details}, please try again"
        );
        Self::create(
            status::CONFLICT,
            &format!("error.msg.platform.service.{type_}.conflict"),
            &msg,
            &msg,
            Vec::new(),
        )
    }

    /// `unAuthorized(defaultUserMessage)` — 403.
    #[must_use]
    pub fn un_authorized(default_user_message: &str) -> Self {
        let code = "error.msg.not.authorized";
        Self::create(
            status::FORBIDDEN,
            code,
            "The user associated with credentials passed on this request does not have sufficient privileges to perform this action.",
            "Insufficient privileges to perform this action.",
            vec![ApiParameterError::general_error(
                code,
                default_user_message,
                &[],
            )],
        )
    }

    /// `domainRuleViolation(code, defaultUserMessage, args...)` — 403.
    #[must_use]
    pub fn domain_rule_violation(code: &str, default_user_message: &str, args: &[String]) -> Self {
        Self::create(
            status::FORBIDDEN,
            "validation.msg.domain.rule.violation",
            "Request was understood but caused a domain rule violation.",
            "Errors contain reason for domain rule violation.",
            vec![ApiParameterError::general_error(
                code,
                default_user_message,
                args,
            )],
        )
    }

    /// `notImplemented(code, defaultUserMessage, args...)` — 501.
    #[must_use]
    pub fn not_implemented(code: &str, default_user_message: &str, args: &[String]) -> Self {
        Self::create(
            status::NOT_IMPLEMENTED,
            "validation.msg.not.implemented",
            "Request was understood but not implemented.",
            "Errors contain reason for not implemented violation.",
            vec![ApiParameterError::general_error(
                code,
                default_user_message,
                args,
            )],
        )
    }

    /// `dataIntegrityError(code, defaultUserMessage, parameterName, args...)` — 403.
    #[must_use]
    pub fn data_integrity_error(
        code: &str,
        default_user_message: &str,
        parameter_name: &str,
        args: &[String],
    ) -> Self {
        let developer_message =
            "The request caused a data integrity issue to be fired by the database.";
        Self::create(
            status::FORBIDDEN,
            code,
            developer_message,
            default_user_message,
            vec![ApiParameterError::parameter_error(
                code,
                developer_message,
                default_user_message,
                parameter_name,
                args,
            )],
        )
    }

    /// `notFound(code, defaultUserMessage, args...)` — 404.
    #[must_use]
    pub fn not_found(code: &str, default_user_message: &str, args: &[String]) -> Self {
        let msg = "The requested resource is not available.";
        Self::create(
            status::NOT_FOUND,
            "error.msg.resource.not.found",
            msg,
            msg,
            vec![ApiParameterError::resource_identifier_not_found(
                code,
                default_user_message,
                args,
            )],
        )
    }

    /// `badClientRequest(code, defaultUserMessage, errors)` — 400.
    #[must_use]
    pub fn bad_client_request(
        code: &str,
        default_user_message: &str,
        errors: Vec<ApiParameterError>,
    ) -> Self {
        Self::create(
            status::BAD_REQUEST,
            code,
            "The request was invalid. This typically will happen due to validation errors which are provided.",
            default_user_message,
            errors,
        )
    }

    /// `serverSideError(code, defaultUserMessage, args...)` — 500.
    #[must_use]
    pub fn server_side_error(code: &str, default_user_message: &str, args: &[String]) -> Self {
        let msg = "An unexpected error occured on the platform server.";
        Self::create(
            status::INTERNAL_SERVER_ERROR,
            "error.msg.platform.server.side.error",
            msg,
            msg,
            vec![ApiParameterError::general_error(
                code,
                default_user_message,
                args,
            )],
        )
    }

    /// `serviceUnavailable(code, defaultUserMessage, args...)` — 503.
    #[must_use]
    pub fn service_unavailable(code: &str, default_user_message: &str, args: &[String]) -> Self {
        let msg =
            "The server is currently unable to handle the request , please try after some time.";
        Self::create(
            status::SERVICE_UNAVAILABLE,
            "error.msg.platform.service.unavailable",
            msg,
            msg,
            vec![ApiParameterError::general_error(
                code,
                default_user_message,
                args,
            )],
        )
    }
}
