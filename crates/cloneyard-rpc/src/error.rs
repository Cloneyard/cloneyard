use std::fmt;

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

    pub(crate) fn internal(detail: impl fmt::Display) -> Self {
        Self::new(Self::INTERNAL_ERROR, "Internal error").with_data(detail.to_string())
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
