use cloneyard_api::MethodError;
use cloneyard_api::account::{Account, AddError, AddParams, Provider};
use serde_json::{Value, json};
use ulid::Ulid;

fn errors() -> [AddError; 4] {
    [
        AddError::DirNotAbsolute {
            dir: "claude".to_owned(),
        },
        AddError::NotADirectory {
            dir: "/tmp/missing".to_owned(),
        },
        AddError::DirTaken {
            dir: "/data/accounts/work/claude".to_owned(),
            account: "work".to_owned(),
        },
        AddError::NameTaken {
            name: "personal".to_owned(),
        },
    ]
}

#[test]
fn each_error_of_account_add_has_its_own_code_and_its_fields_as_data() {
    let encoded: Vec<(i64, &str, Value)> = errors()
        .iter()
        .map(|error| {
            let (code, data) = error.encode().unwrap();
            (code.code, code.message, data)
        })
        .collect();

    assert_eq!(
        encoded,
        [
            (
                1100,
                "Directory is not an absolute path",
                json!({"dir": "claude"})
            ),
            (
                1101,
                "Path is not a directory",
                json!({"dir": "/tmp/missing"})
            ),
            (
                1102,
                "Directory belongs to another account",
                json!({"dir": "/data/accounts/work/claude", "account": "work"})
            ),
            (1103, "Account name is taken", json!({"name": "personal"})),
        ]
    );
}

#[test]
fn each_error_of_account_add_decodes_from_its_code_and_data() {
    for error in errors() {
        let (code, data) = error.encode().unwrap();

        let decoded = AddError::decode(code.code, Some(data)).unwrap().unwrap();

        assert_eq!(decoded, error);
    }
}

#[test]
fn the_errors_of_account_add_name_the_directory_or_the_account() {
    let messages: Vec<String> = errors().iter().map(ToString::to_string).collect();

    assert_eq!(
        messages,
        [
            "claude is not an absolute path",
            "/tmp/missing is not a directory",
            "/data/accounts/work/claude is already the directory of account work",
            "account personal already exists",
        ]
    );
}

#[test]
fn the_provider_goes_by_its_cli_name_on_the_wire_and_in_print() {
    let params = AddParams {
        name: "personal".to_owned(),
        provider: Provider::ClaudeCode,
        dir: "/data/accounts/personal/claude".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({
            "name": "personal",
            "provider": "claude-code",
            "dir": "/data/accounts/personal/claude"
        })
    );
    assert_eq!(Provider::ClaudeCode.to_string(), "claude-code");
}

#[test]
fn an_account_without_a_directory_on_this_node_has_a_null_dir() {
    let id = Ulid::generate();
    let account = Account {
        id,
        name: "personal".to_owned(),
        provider: Provider::ClaudeCode,
        dir: None,
    };

    assert_eq!(
        serde_json::to_value(account).unwrap(),
        json!({
            "id": id.to_string(),
            "name": "personal",
            "provider": "claude-code",
            "dir": null
        })
    );
}
