use std::fmt;

use cloneyard_api::MethodError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Error {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Error {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;

    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: impl Into<Value>) -> Self {
        self.data = Some(data.into());
        self
    }

    pub(crate) fn parse_error(detail: impl fmt::Display) -> Self {
        Self::new(Self::PARSE_ERROR, "Parse error").with_data(detail.to_string())
    }

    pub(crate) fn invalid_request(detail: impl fmt::Display) -> Self {
        Self::new(Self::INVALID_REQUEST, "Invalid Request").with_data(detail.to_string())
    }

    pub(crate) fn method_not_found(method: &str) -> Self {
        Self::new(Self::METHOD_NOT_FOUND, "Method not found").with_data(method)
    }

    pub(crate) fn invalid_params(detail: impl fmt::Display) -> Self {
        Self::new(Self::INVALID_PARAMS, "Invalid params").with_data(detail.to_string())
    }

    /// -32603, for a failure that is not one of the method's own errors.
    #[must_use]
    pub fn internal(detail: impl fmt::Display) -> Self {
        Self::new(Self::INTERNAL_ERROR, "Internal error").with_data(detail.to_string())
    }

    pub(crate) fn method<E: MethodError>(error: &E) -> Self {
        match error.encode() {
            Some((code, fields)) => Self::new(code.code, code.message).with_data(fields),
            None => Self::internal(format!("no error code for {error:?}")),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)?;
        match &self.data {
            Some(Value::String(detail)) => write!(f, ": {detail}"),
            Some(data) => write!(f, ": {data}"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use cloneyard_api::{ErrorCode, MethodError};
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    use super::Error;

    #[derive(Debug, Serialize, Deserialize, JsonSchema, thiserror::Error)]
    enum Unlisted {
        #[error("lost {name}")]
        Lost { name: String },
    }

    impl MethodError for Unlisted {
        const ERRORS: &'static [ErrorCode] = &[];
    }

    #[test]
    fn sends_a_method_error_without_a_code_as_an_internal_error() {
        let error = Error::method(&Unlisted::Lost {
            name: "app".to_owned(),
        });

        assert_eq!(
            error,
            Error::new(Error::INTERNAL_ERROR, "Internal error")
                .with_data("no error code for Lost { name: \"app\" }")
        );
    }
}
