//! The methods of the Cloneyard API, each with the types of its params and its result, and the OpenRPC document built from them.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::rpc::{Components, ContentDescriptor, Document, Info, MethodObject};

pub mod rpc;
pub mod system;

/// A method of the API: its name, what it does, and the types of its params and its result.
pub trait Method {
    const NAME: &'static str;
    /// One line on what the method does.
    const SUMMARY: Option<&'static str> = None;
    /// A longer explanation, when one line is not enough.
    const DESCRIPTION: Option<&'static str> = None;
    /// `()` for a method without params.
    type Params: Serialize + DeserializeOwned + JsonSchema;
    type Result: Serialize + DeserializeOwned + JsonSchema;
}

/// The OpenRPC document of the API, built from the types of its methods.
///
/// # Panics
///
/// When a method takes params, which the document can't describe yet.
#[must_use]
pub fn document() -> Document {
    let mut generator = SchemaSettings::draft07()
        .with(|settings| settings.definitions_path = "/components/schemas".into())
        .into_generator();
    let methods = vec![
        method::<system::Ping>(&mut generator),
        method::<system::Version>(&mut generator),
        method::<rpc::Discover>(&mut generator),
    ];
    Document {
        openrpc: "1.4.1".to_owned(),
        info: Info {
            title: "Cloneyard".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        methods,
        components: Components {
            schemas: generator.take_definitions(true),
        },
    }
}

fn method<M: Method>(generator: &mut SchemaGenerator) -> MethodObject {
    assert_eq!(
        generator.subschema_for::<M::Params>(),
        json_schema!({"type": "null"}),
        "{} takes params, which need by-name content descriptors in the document",
        M::NAME
    );
    MethodObject {
        name: M::NAME.to_owned(),
        summary: M::SUMMARY.map(str::to_owned),
        description: M::DESCRIPTION.map(str::to_owned),
        params: Vec::new(),
        result: ContentDescriptor {
            name: M::Result::schema_name().into_owned(),
            schema: generator.subschema_for::<M::Result>(),
        },
    }
}

// Crockford base32 without I, L, O and U; 26 characters hold 130 bits, so a 128-bit id starts with 0-7.
fn ulid_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": "^[0-7][0-9A-HJKMNP-TV-Z]{25}$",
    })
}
