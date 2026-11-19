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
fn lists_every_method_and_only_project_add_takes_params() {
    let document = document();
    let methods = document["methods"].as_array().unwrap();

    let names: Vec<&Value> = methods.iter().map(|method| &method["name"]).collect();
    assert_eq!(
        names,
        [
            "system.ping",
            "system.version",
            "rpc.discover",
            "project.add",
            "project.list"
        ]
    );
    for method in methods
        .iter()
        .filter(|method| method["name"] != "project.add")
    {
        assert_eq!(method["params"], json!([]), "{method}");
    }
}

#[test]
fn describes_the_params_of_project_add_by_name() {
    let document = document();
    let add = method(&document, "project.add");

    assert_eq!(add["paramStructure"], "by-name");
    let params: Vec<(&Value, &Value, &Value)> = add["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|param| (&param["name"], &param["required"], &param["schema"]["type"]))
        .collect();
    assert_eq!(
        params,
        [
            (&json!("name"), &Value::Null, &json!(["string", "null"])),
            (&json!("path"), &json!(true), &json!("string")),
        ]
    );
}

#[test]
fn lists_the_errors_of_project_add_with_their_codes_and_fields() {
    let document = document();

    let errors: Vec<(&Value, &Value, &Value)> = method(&document, "project.add")["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| {
            (
                &error["code"],
                &error["message"],
                &error["data"]["required"],
            )
        })
        .collect();

    assert_eq!(
        errors,
        [
            (
                &json!(1000),
                &json!("Path is not absolute"),
                &json!(["path"])
            ),
            (
                &json!(1001),
                &json!("Path is not in a git working tree"),
                &json!(["path"])
            ),
            (
                &json!(1002),
                &json!("Repo is already in a project"),
                &json!(["path", "project"])
            ),
            (
                &json!(1003),
                &json!("Project name is taken"),
                &json!(["name"])
            ),
        ]
    );
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
    assert_eq!(
        method(&document, "project.add")["result"]["schema"],
        json!({"$ref": "#/components/schemas/Project"})
    );
    let schemas: Vec<&String> = document["components"]["schemas"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(
        schemas,
        [
            "ListResult",
            "PingResult",
            "Project",
            "Repo",
            "VersionResult"
        ]
    );
}

#[test]
fn leaves_no_schema_or_definitions_key() {
    let document = document();
    let mut found = Vec::new();

    keys_named(&document, &["$schema", "definitions"], &mut found);

    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn describes_a_method_without_params_or_errors_without_a_param_structure_or_errors() {
    let document = document();

    assert_eq!(
        *method(&document, "system.ping"),
        json!({
            "name": "system.ping",
            "summary": "Returns the node id, to check that the daemon answers",
            "params": [],
            "result": {
                "name": "PingResult",
                "schema": {"$ref": "#/components/schemas/PingResult"}
            }
        })
    );
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
