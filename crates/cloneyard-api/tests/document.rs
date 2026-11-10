use serde_json::{Value, json};

fn document() -> Value {
    serde_json::to_value(cloneyard_api::document()).unwrap()
}

fn method<'a>(document: &'a Value, name: &str) -> &'a Value {
    document["methods"]
        .as_array()
        .unwrap()
        .iter()
        .find(|method| method["name"] == name)
        .unwrap_or_else(|| panic!("no method {name}"))
}

fn keys_named<'a>(value: &'a Value, names: &[&str], found: &mut Vec<&'a str>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if names.contains(&key.as_str()) {
                    found.push(key);
                }
                keys_named(value, names, found);
            }
        }
        Value::Array(items) => {
            for item in items {
                keys_named(item, names, found);
            }
        }
        _ => {}
    }
}

#[test]
fn is_an_openrpc_1_4_1_document_with_the_crate_version() {
    let document = document();

    assert_eq!(document["openrpc"], "1.4.1");
    assert_eq!(document["info"]["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn lists_every_method_without_params() {
    let document = document();
    let methods = document["methods"].as_array().unwrap();

    let names: Vec<&Value> = methods.iter().map(|method| &method["name"]).collect();
    assert_eq!(names, ["system.ping", "system.version", "rpc.discover"]);
    for method in methods {
        assert_eq!(method["params"], json!([]), "{method}");
    }
}

#[test]
fn results_refer_to_shared_schemas_in_components() {
    let document = document();

    assert_eq!(
        method(&document, "system.ping")["result"]["schema"],
        json!({"$ref": "#/components/schemas/PingResult"})
    );
    assert_eq!(
        method(&document, "system.version")["result"]["schema"],
        json!({"$ref": "#/components/schemas/VersionResult"})
    );
    let schemas: Vec<&String> = document["components"]["schemas"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(schemas, ["PingResult", "VersionResult"]);
}

#[test]
fn leaves_no_schema_or_definitions_key() {
    let document = document();
    let mut found = Vec::new();

    keys_named(&document, &["$schema", "definitions"], &mut found);

    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn describes_rpc_discover_as_the_openrpc_spec_does() {
    let document = document();

    assert_eq!(
        *method(&document, "rpc.discover"),
        json!({
            "name": "rpc.discover",
            "description": "Returns an OpenRPC schema as a description of this service",
            "params": [],
            "result": {
                "name": "OpenRPC Schema",
                "schema": {
                    "$ref": "https://raw.githubusercontent.com/open-rpc/meta-schema/master/schema.json"
                }
            }
        })
    );
}
