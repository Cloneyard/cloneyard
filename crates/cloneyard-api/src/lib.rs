//! The methods of the Cloneyard API, each with the types of its params and its result.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub mod system;

/// A method of the API: its name, what it does, and the types of its params and its result.
pub trait Method {
    const NAME: &'static str;
    const SUMMARY: &'static str;
    /// `()` for a method without params.
    type Params: Serialize + DeserializeOwned + JsonSchema;
    type Result: Serialize + DeserializeOwned + JsonSchema;
}

// Crockford base32 without I, L, O and U; 26 characters hold 130 bits, so a 128-bit id starts with 0-7.
fn ulid_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": "^[0-7][0-9A-HJKMNP-TV-Z]{25}$",
    })
}
