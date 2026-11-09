use serde::de::{Error as _, Unexpected};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};

use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum Id {
    Number(Number),
    String(String),
    Null,
}

/// The `"jsonrpc": "2.0"` member.
#[derive(Debug, Clone, Copy)]
pub(crate) struct V2;

impl Serialize for V2 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for V2 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = String::deserialize(deserializer)?;
        if version == "2.0" {
            Ok(V2)
        } else {
            Err(D::Error::invalid_value(
                Unexpected::Str(&version),
                &"\"2.0\"",
            ))
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Request {
    pub jsonrpc: V2,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// `None` marks a notification, while `Some(Id::Null)` is a request with a null id.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<Id>,
}

fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Id>, D::Error> {
    Id::deserialize(deserializer).map(Some)
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Response {
    pub jsonrpc: V2,
    #[serde(flatten)]
    pub outcome: Outcome,
    pub id: Id,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Outcome {
    Result(Value),
    Error(Error),
}

impl Response {
    pub fn new(id: Id, result: Result<Value, Error>) -> Self {
        let outcome = match result {
            Ok(value) => Outcome::Result(value),
            Err(error) => Outcome::Error(error),
        };
        Self {
            jsonrpc: V2,
            outcome,
            id,
        }
    }
}

/// Parses one incoming line; on failure returns the id to answer with, if one could be read.
pub(crate) fn parse_request(line: &[u8]) -> Result<Request, (Id, Error)> {
    let value: Value =
        serde_json::from_slice(line).map_err(|error| (Id::Null, Error::parse_error(error)))?;
    if !value.is_object() {
        return Err((
            Id::Null,
            Error::invalid_request("expected a request object"),
        ));
    }
    let id = value
        .get("id")
        .and_then(|id| Id::deserialize(id).ok())
        .unwrap_or(Id::Null);
    // Checked on the raw value, because serde reads `"params": null` as no params.
    if value
        .get("params")
        .is_some_and(|params| !params.is_array() && !params.is_object())
    {
        return Err((
            id,
            Error::invalid_request("params must be an array or an object"),
        ));
    }
    serde_json::from_value(value).map_err(|error| (id, Error::invalid_request(error)))
}
