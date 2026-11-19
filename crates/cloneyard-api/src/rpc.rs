use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Method, NoError};

/// `rpc.discover`, as the OpenRPC spec defines it.
pub struct Discover;

impl Method for Discover {
    const NAME: &'static str = "rpc.discover";
    const DESCRIPTION: Option<&'static str> =
        Some("Returns an OpenRPC schema as a description of this service");
    type Params = ();
    type Result = Document;
    type Error = NoError;
}

/// An OpenRPC document, with its fields in the order of the spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub openrpc: String,
    pub info: Info,
    pub methods: Vec<MethodObject>,
    pub components: Components,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub title: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodObject {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub params: Vec<ContentDescriptor>,
    pub result: ContentDescriptor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorObject>,
    #[serde(
        default,
        rename = "paramStructure",
        skip_serializing_if = "Option::is_none"
    )]
    pub param_structure: Option<ParamStructure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentDescriptor {
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    pub schema: Schema,
}

/// An error a method returns of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    /// The schema of the fields the error carries, since OpenRPC has no other place for one.
    pub data: Schema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParamStructure {
    ByName,
    ByPosition,
    Either,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Components {
    pub schemas: Map<String, Value>,
}

impl JsonSchema for Document {
    fn inline_schema() -> bool {
        true
    }

    // The spec gives this name to the result of rpc.discover.
    fn schema_name() -> Cow<'static, str> {
        "OpenRPC Schema".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "$ref": "https://raw.githubusercontent.com/open-rpc/meta-schema/master/schema.json",
        })
    }
}
