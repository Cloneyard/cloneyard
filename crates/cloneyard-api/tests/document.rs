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
fn lists_every_method_and_which_of_them_take_params() {
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
            "project.list",
            "account.add",
            "account.list",
            "clone.run",
            "clone.list",
            "clone.show",
            "clone.stop",
            "clone.events"
        ]
    );
    let with_params: Vec<&Value> = methods
        .iter()
        .filter(|method| method["params"] != json!([]))
        .map(|method| &method["name"])
        .collect();
    assert_eq!(
        with_params,
        [
            "project.add",
            "account.add",
            "clone.run",
            "clone.show",
            "clone.stop",
            "clone.events"
        ]
    );
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
fn describes_the_params_of_account_add_by_name() {
    let document = document();
    let add = method(&document, "account.add");

    assert_eq!(add["paramStructure"], "by-name");
    let params: Vec<(&Value, &Value, &Value)> = add["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|param| (&param["name"], &param["required"], &param["schema"]))
        .collect();
    assert_eq!(
        params,
        [
            (
                &json!("dir"),
                &json!(true),
                &json!({
                    "description": "The absolute path of the directory that holds the account's login on this node.",
                    "type": "string"
                })
            ),
            (
                &json!("name"),
                &json!(true),
                &json!({"description": "The name of the account.", "type": "string"})
            ),
            (
                &json!("provider"),
                &json!(true),
                &json!({"$ref": "#/components/schemas/Provider"})
            ),
        ]
    );
}

#[test]
fn lists_the_errors_of_account_add_with_their_codes_and_fields() {
    let document = document();

    let errors: Vec<(&Value, &Value, &Value)> = method(&document, "account.add")["errors"]
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
                &json!(1100),
                &json!("Directory is not an absolute path"),
                &json!(["dir"])
            ),
            (
                &json!(1101),
                &json!("Path is not a directory"),
                &json!(["dir"])
            ),
            (
                &json!(1102),
                &json!("Directory belongs to another account"),
                &json!(["dir", "account"])
            ),
            (
                &json!(1103),
                &json!("Account name is taken"),
                &json!(["name"])
            ),
        ]
    );
}

#[test]
fn describes_the_params_of_clone_run_by_name() {
    let document = document();

    let params: Vec<(&Value, &Value)> = method(&document, "clone.run")["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|param| (&param["name"], &param["required"]))
        .collect();

    assert_eq!(
        params,
        [
            (&json!("account"), &json!(true)),
            (&json!("project"), &json!(true)),
            (&json!("prompt"), &json!(true)),
        ]
    );
}

#[test]
fn lists_the_errors_of_the_clone_methods_with_their_codes_and_fields() {
    let document = document();
    let errors = |name: &str| -> Vec<(Value, Value, Value)> {
        method(&document, name)["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| {
                (
                    error["code"].clone(),
                    error["message"].clone(),
                    error["data"]["required"].clone(),
                )
            })
            .collect()
    };

    assert_eq!(
        errors("clone.run"),
        [
            (json!(1200), json!("No such project"), json!(["name"])),
            (json!(1201), json!("No such account"), json!(["name"])),
            (
                json!(1204),
                json!("No sandbox for clones on this node"),
                json!(["config"])
            ),
        ]
    );
    assert_eq!(
        errors("clone.show"),
        [(json!(1202), json!("No such clone"), json!(["handle"]))]
    );
    assert_eq!(
        errors("clone.stop"),
        [
            (json!(1202), json!("No such clone"), json!(["handle"])),
            (
                json!(1203),
                json!("Clone is not running"),
                json!(["handle", "state"])
            ),
        ]
    );
    assert_eq!(
        errors("clone.events"),
        [(json!(1202), json!("No such clone"), json!(["handle"]))]
    );
}

#[test]
fn describes_a_handle_as_ct_and_at_least_four_digits() {
    let document = document();

    assert_eq!(
        document["components"]["schemas"]["Handle"],
        json!({
            "description": "A clone's short name on its node, such as `CT-0042`.",
            "type": "string",
            "pattern": "^CT-[0-9]{4,}$"
        })
    );
    assert_eq!(
        method(&document, "clone.show")["params"][0]["schema"],
        json!({"$ref": "#/components/schemas/Handle"})
    );
}

#[test]
fn names_the_provider_by_its_cli_name() {
    let document = document();

    assert_eq!(
        document["components"]["schemas"]["Provider"]["oneOf"],
        json!([{
            "const": "claude-code",
            "description": "Claude Code, which keeps each login in a config directory of its own.",
            "type": "string"
        }])
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
    assert_eq!(
        method(&document, "account.list")["result"]["schema"],
        json!({"$ref": "#/components/schemas/AccountListResult"})
    );
    let schemas: Vec<&String> = document["components"]["schemas"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(
        schemas,
        [
            "Account",
            "AccountListResult",
            "Clone",
            "CloneEventsResult",
            "CloneListResult",
            "Handle",
            "HookEvent",
            "ListResult",
            "Mode",
            "PingResult",
            "Project",
            "Provider",
            "Repo",
            "Session",
            "State",
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
