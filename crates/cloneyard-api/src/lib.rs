//! The methods of the Cloneyard API, each with the types of its params, its result and its errors, and the OpenRPC document built from them.

use std::fmt;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::rpc::{
    Components, ContentDescriptor, Document, ErrorObject, Info, MethodObject, ParamStructure,
};

pub mod account;
pub mod clone;
pub mod project;
pub mod rpc;
pub mod system;

/// A method of the API: its name, what it does, and the types of its params, its result and its errors.
pub trait Method {
    const NAME: &'static str;
    /// One line on what the method does.
    const SUMMARY: Option<&'static str> = None;
    /// A longer explanation, when one line is not enough.
    const DESCRIPTION: Option<&'static str> = None;
    /// `()` for a method without params, or a struct whose fields are the params by name.
    type Params: Serialize + DeserializeOwned + JsonSchema;
    type Result: Serialize + DeserializeOwned + JsonSchema;
    /// [`NoError`] for a method that fails only the way any method can.
    type Error: MethodError;
}

/// The errors of a method of its own, an enum whose variants have fields.
///
/// Each variant has a code from 1000 up, a block of 100 per namespace, and its fields go in the `data` of the JSON-RPC error object.
pub trait MethodError:
    std::error::Error + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
    /// Every variant, by its name in serde.
    const ERRORS: &'static [ErrorCode];

    /// The code of this error and its fields, or `None` when its variant has no fields or no code.
    #[must_use]
    fn encode(&self) -> Option<(&'static ErrorCode, Value)> {
        let Ok(Value::Object(variant)) = serde_json::to_value(self) else {
            return None;
        };
        let (name, fields) = variant.into_iter().next()?;
        let code = Self::ERRORS.iter().find(|error| error.variant == name)?;
        Some((code, fields))
    }

    /// The error with `code` and the fields in `data`, or `None` when `code` is not one of this method's.
    #[must_use]
    fn decode(code: i64, data: Option<Value>) -> Option<serde_json::Result<Self>> {
        let error = Self::ERRORS.iter().find(|error| error.code == code)?;
        let variant = Map::from_iter([(error.variant.to_owned(), data.unwrap_or_default())]);
        Some(serde_json::from_value(Value::Object(variant)))
    }
}

/// The code of one variant of a [`MethodError`], with the message that goes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCode {
    pub variant: &'static str,
    pub code: i64,
    /// A short description that holds for every error with this code, such as `Project name is taken`.
    pub message: &'static str,
}

/// The errors of a method that has none of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum NoError {}

impl fmt::Display for NoError {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}

impl std::error::Error for NoError {}

impl MethodError for NoError {
    const ERRORS: &'static [ErrorCode] = &[];
}

/// The OpenRPC document of the API, built from the types of its methods.
///
/// # Panics
///
/// When the params of a method are neither `()` nor a struct, or an error in [`MethodError::ERRORS`] is not a variant with fields.
#[must_use]
pub fn document() -> Document {
    let mut generator = SchemaSettings::draft07()
        .with(|settings| settings.definitions_path = "/components/schemas".into())
        .into_generator();
    let methods = vec![
        method::<system::Ping>(&mut generator),
        method::<system::Version>(&mut generator),
        method::<rpc::Discover>(&mut generator),
        method::<project::Add>(&mut generator),
        method::<project::List>(&mut generator),
        method::<account::Add>(&mut generator),
        method::<account::List>(&mut generator),
        method::<clone::Run>(&mut generator),
        method::<clone::List>(&mut generator),
        method::<clone::Show>(&mut generator),
        method::<clone::Stop>(&mut generator),
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
    let params = params(&M::Params::json_schema(generator), M::NAME);
    MethodObject {
        name: M::NAME.to_owned(),
        summary: M::SUMMARY.map(str::to_owned),
        description: M::DESCRIPTION.map(str::to_owned),
        param_structure: (!params.is_empty()).then_some(ParamStructure::ByName),
        params,
        result: ContentDescriptor {
            name: M::Result::schema_name().into_owned(),
            schema: generator.subschema_for::<M::Result>(),
            required: false,
        },
        errors: errors::<M::Error>(generator),
    }
}

fn params(schema: &Schema, method: &str) -> Vec<ContentDescriptor> {
    if *schema == json_schema!({"type": "null"}) {
        return Vec::new();
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the params of {method} are not a struct"));
    let required = schema.get("required").and_then(Value::as_array);
    properties
        .iter()
        .map(|(name, schema)| ContentDescriptor {
            name: name.clone(),
            schema: schema_of(schema),
            required: required.is_some_and(|required| required.contains(&name.as_str().into())),
        })
        .collect()
}

fn errors<E: MethodError>(generator: &mut SchemaGenerator) -> Vec<ErrorObject> {
    let schema = E::json_schema(generator);
    E::ERRORS
        .iter()
        .map(|error| ErrorObject {
            code: error.code,
            message: error.message.to_owned(),
            data: fields_of(&schema, error.variant),
        })
        .collect()
}

// serde writes a variant as `{"<variant>": <fields>}`, which schemars puts in `oneOf`.
fn fields_of(schema: &Schema, variant: &str) -> Schema {
    let fields = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|one| one.get("properties")?.get(variant))
        .unwrap_or_else(|| panic!("no variant {variant} with fields"));
    schema_of(fields)
}

fn schema_of(value: &Value) -> Schema {
    Schema::try_from(value.clone()).expect("schemars gives an object or a bool")
}

// Crockford base32 without I, L, O and U; 26 characters hold 130 bits, so a 128-bit id starts with 0-7.
fn ulid_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": "^[0-7][0-9A-HJKMNP-TV-Z]{25}$",
    })
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use schemars::JsonSchema;
    use schemars::generate::SchemaSettings;
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    use super::{ErrorCode, Method, MethodError, method};

    struct Rename;

    impl Method for Rename {
        const NAME: &'static str = "test.rename";
        type Params = RenameParams;
        type Result = ();
        type Error = RenameError;
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct RenameParams {
        /// The current name.
        from: String,
        /// The new name.
        to: Option<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    enum RenameError {
        Missing { name: String },
        Taken { name: String, id: u32 },
    }

    impl fmt::Display for RenameError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Missing { name } => write!(f, "no project {name}"),
                Self::Taken { name, .. } => write!(f, "project {name} already exists"),
            }
        }
    }

    impl std::error::Error for RenameError {}

    impl MethodError for RenameError {
        const ERRORS: &'static [ErrorCode] = &[
            ErrorCode {
                variant: "Missing",
                code: 1000,
                message: "No such project",
            },
            ErrorCode {
                variant: "Taken",
                code: 1001,
                message: "Project name is taken",
            },
        ];
    }

    fn rename() -> Value {
        let mut generator = SchemaSettings::draft07().into_generator();
        serde_json::to_value(method::<Rename>(&mut generator)).unwrap()
    }

    #[test]
    fn describes_params_by_name_and_marks_the_required_ones() {
        let rename = rename();

        assert_eq!(rename["paramStructure"], "by-name");
        assert_eq!(
            rename["params"],
            json!([
                {
                    "name": "from",
                    "required": true,
                    "schema": {"description": "The current name.", "type": "string"}
                },
                {
                    "name": "to",
                    "schema": {"description": "The new name.", "type": ["string", "null"]}
                }
            ])
        );
    }

    #[test]
    fn lists_each_error_with_its_code_message_and_the_schema_of_its_fields() {
        assert_eq!(
            rename()["errors"],
            json!([
                {
                    "code": 1000,
                    "message": "No such project",
                    "data": {
                        "type": "object",
                        "properties": {"name": {"type": "string"}},
                        "required": ["name"]
                    }
                },
                {
                    "code": 1001,
                    "message": "Project name is taken",
                    "data": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "integer", "format": "uint32", "minimum": 0},
                            "name": {"type": "string"}
                        },
                        "required": ["name", "id"]
                    }
                }
            ])
        );
    }

    #[test]
    fn encodes_an_error_as_its_code_and_message_with_its_fields() {
        let error = RenameError::Taken {
            name: "app".to_owned(),
            id: 7,
        };

        let (code, data) = error.encode().unwrap();

        assert_eq!((code.code, code.message), (1001, "Project name is taken"));
        assert_eq!(data, json!({"name": "app", "id": 7}));
    }

    #[test]
    fn decodes_an_error_from_its_code_and_fields() {
        let decoded = RenameError::decode(1000, Some(json!({"name": "app"})));

        assert_eq!(
            decoded.unwrap().unwrap(),
            RenameError::Missing {
                name: "app".to_owned()
            }
        );
    }

    #[test]
    fn decodes_no_error_from_a_code_that_is_not_its_own() {
        assert!(RenameError::decode(-32603, Some(json!("boom"))).is_none());
    }

    #[test]
    fn fails_to_decode_fields_that_do_not_fit_the_code() {
        let decoded = RenameError::decode(1001, Some(json!({"name": "app"})));

        assert!(decoded.unwrap().is_err());
    }
}
