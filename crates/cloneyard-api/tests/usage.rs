use cloneyard_api::MethodError;
use cloneyard_api::usage::{ToolUsage, ToolsError, ToolsParams, ToolsResult};
use serde_json::json;

#[test]
fn an_unknown_project_has_its_own_code_and_names_the_project() {
    let error = ToolsError::UnknownProject {
        name: "app".to_owned(),
    };

    let (code, data) = error.encode().unwrap();

    assert_eq!((code.code, code.message), (1300, "No such project"));
    assert_eq!(data, json!({"name": "app"}));
    assert_eq!(error.to_string(), "no project app");
}

#[test]
fn the_params_leave_out_a_project_they_have_none_of() {
    let every = ToolsParams {
        since: 1_764_000_000_000,
        project: None,
    };
    let one = ToolsParams {
        project: Some("app".to_owned()),
        ..every.clone()
    };

    let json = [every, one].map(|params| serde_json::to_value(params).unwrap());

    assert_eq!(
        json,
        [
            json!({"since": 1_764_000_000_000_i64}),
            json!({"since": 1_764_000_000_000_i64, "project": "app"}),
        ]
    );
}

#[test]
fn a_tool_without_a_command_path_or_result_size_goes_on_the_wire_with_nulls() {
    let result = ToolsResult {
        tools: vec![
            ToolUsage {
                tool: "Bash".to_owned(),
                command: Some("gh api".to_owned()),
                path: None,
                calls: 41,
                average_result_tokens: Some(11_200),
                errors: 6,
                retries: 5,
                denials: 0,
            },
            ToolUsage {
                tool: "TodoWrite".to_owned(),
                command: None,
                path: None,
                calls: 2,
                average_result_tokens: None,
                errors: 0,
                retries: 0,
                denials: 2,
            },
        ],
    };

    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(
        json,
        json!({"tools": [
            {
                "tool": "Bash",
                "command": "gh api",
                "path": null,
                "calls": 41,
                "average_result_tokens": 11_200,
                "errors": 6,
                "retries": 5,
                "denials": 0,
            },
            {
                "tool": "TodoWrite",
                "command": null,
                "path": null,
                "calls": 2,
                "average_result_tokens": null,
                "errors": 0,
                "retries": 0,
                "denials": 2,
            },
        ]})
    );
    assert_eq!(serde_json::from_value::<ToolsResult>(json).unwrap(), result);
}
