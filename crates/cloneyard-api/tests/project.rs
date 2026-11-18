use cloneyard_api::MethodError;
use cloneyard_api::project::{AddError, AddParams};
use serde_json::{Value, json};

fn errors() -> [AddError; 4] {
    [
        AddError::PathNotAbsolute {
            path: "app".to_owned(),
        },
        AddError::NotInWorkTree {
            path: "/tmp/notes".to_owned(),
        },
        AddError::RepoTaken {
            path: "/code/app".to_owned(),
            project: "web".to_owned(),
        },
        AddError::NameTaken {
            name: "app".to_owned(),
        },
    ]
}

#[test]
fn each_error_of_project_add_has_its_own_code_and_its_fields_as_data() {
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
            (1000, "Path is not absolute", json!({"path": "app"})),
            (
                1001,
                "Path is not in a git working tree",
                json!({"path": "/tmp/notes"})
            ),
            (
                1002,
                "Repo is already in a project",
                json!({"path": "/code/app", "project": "web"})
            ),
            (1003, "Project name is taken", json!({"name": "app"})),
        ]
    );
}

#[test]
fn each_error_of_project_add_decodes_from_its_code_and_data() {
    for error in errors() {
        let (code, data) = error.encode().unwrap();

        let decoded = AddError::decode(code.code, Some(data)).unwrap().unwrap();

        assert_eq!(decoded, error);
    }
}

#[test]
fn the_errors_of_project_add_name_the_path_or_the_project() {
    let messages: Vec<String> = errors().iter().map(ToString::to_string).collect();

    assert_eq!(
        messages,
        [
            "app is not an absolute path",
            "/tmp/notes is not in a git working tree",
            "/code/app is already in project web",
            "project app already exists",
        ]
    );
}

#[test]
fn the_params_of_project_add_leave_out_a_missing_name() {
    let params = AddParams {
        path: "/code/app".to_owned(),
        name: None,
    };

    assert_eq!(
        serde_json::to_value(params).unwrap(),
        json!({"path": "/code/app"})
    );
}
