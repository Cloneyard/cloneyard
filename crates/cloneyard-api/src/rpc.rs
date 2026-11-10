use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Method;

/// `rpc.discover`, as the OpenRPC spec defines it.
pub struct Discover;

impl Method for Discover {
    const NAME: &'static str = "rpc.discover";
    const DESCRIPTION: Option<&'static str> =
        Some("Returns an OpenRPC schema as a description of this service");
    type Params = ();
    type Result = Document;
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentDescriptor {
    pub name: String,
    pub schema: Schema,
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
