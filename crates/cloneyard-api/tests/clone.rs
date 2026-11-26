use cloneyard_api::MethodError;
use cloneyard_api::clone::{
    Clone, EventsError, EventsResult, Handle, HandleParams, HookEvent, Mode, RunError, Session,
    State, StopError, Usage,
};
use regex::Regex;
use serde_json::{Value, json};
use ulid::Ulid;

fn handle_pattern() -> Regex {
    let schema = schemars::schema_for!(Handle);
    Regex::new(schema.as_value()["pattern"].as_str().unwrap()).unwrap()
}

#[test]
fn a_handle_prints_as_ct_and_at_least_four_digits() {
    let printed: Vec<String> = [1, 42, 9999, 12345]
        .map(|number| Handle::new(number).to_string())
        .into();

    assert_eq!(printed, ["CT-0001", "CT-0042", "CT-9999", "CT-12345"]);
    for text in &printed {
        assert!(handle_pattern().is_match(text), "{text}");
    }
}

#[test]
fn a_handle_reads_as_people_type_it() {
    for text in ["CT-0042", "ct-42", "Ct-042", "CT-42"] {
        assert_eq!(text.parse::<Handle>(), Ok(Handle::new(42)), "{text}");
    }
}

#[test]
fn a_handle_refuses_what_is_not_ct_and_a_number() {
    for text in [
        "",
        "CT-",
        "CT42",
        "XY-0042",
        "CT-42a",
        "CT--42",
        "CT-+42",
        "CT-99999999999",
    ] {
        let error = text.parse::<Handle>().unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("invalid handle {text:?}: a handle is CT- and a number, such as CT-0042")
        );
    }
}

#[test]
fn a_malformed_handle_in_params_fails_to_deserialize() {
    let parsed: Result<HandleParams, _> = serde_json::from_value(json!({"handle": "CT-x"}));

    assert!(parsed.is_err());
}

#[test]
fn each_clone_error_has_its_own_code_and_its_fields_as_data() {
    let handle = Handle::new(1);
    let encoded: Vec<(i64, Value)> = [
        RunError::UnknownProject {
            name: "app".to_owned(),
        }
        .encode(),
        RunError::UnknownAccount {
            name: "personal".to_owned(),
        }
        .encode(),
        RunError::NoSandbox {
            config: "/home/me/.config/cloneyard/node.toml".to_owned(),
        }
        .encode(),
        StopError::UnknownClone { handle }.encode(),
        StopError::NotRunning {
            handle,
            state: State::Done,
        }
        .encode(),
        EventsError::UnknownClone { handle }.encode(),
    ]
    .into_iter()
    .map(|encoded| {
        let (code, data) = encoded.unwrap();
        (code.code, data)
    })
    .collect();

    assert_eq!(
        encoded,
        [
            (1200, json!({"name": "app"})),
            (1201, json!({"name": "personal"})),
            (
                1204,
                json!({"config": "/home/me/.config/cloneyard/node.toml"})
            ),
            (1202, json!({"handle": "CT-0001"})),
            (1203, json!({"handle": "CT-0001", "state": "done"})),
            (1202, json!({"handle": "CT-0001"})),
        ]
    );
}

#[test]
fn the_clone_errors_name_the_project_account_clone_or_settings() {
    let handle = Handle::new(7);
    let messages = [
        RunError::UnknownProject {
            name: "app".to_owned(),
        }
        .to_string(),
        RunError::UnknownAccount {
            name: "personal".to_owned(),
        }
        .to_string(),
        RunError::NoSandbox {
            config: "/home/me/.config/cloneyard/node.toml".to_owned(),
        }
        .to_string(),
        StopError::UnknownClone { handle }.to_string(),
        StopError::NotRunning {
            handle,
            state: State::Stopped,
        }
        .to_string(),
    ];

    assert_eq!(
        messages,
        [
            "no project app",
            "no account personal",
            "this kernel has no Landlock, so clones can't run in the sandbox; \
             set unsandboxed = true in /home/me/.config/cloneyard/node.toml to run them without it",
            "no clone CT-0007",
            "clone CT-0007 is stopped, not running",
        ]
    );
}

#[test]
fn a_clone_goes_on_the_wire_with_its_handle_mode_and_state_as_text() {
    let id = Ulid::generate();
    let session = Ulid::generate();
    let usage = Usage {
        input_tokens: 2,
        output_tokens: 10,
        cache_read_tokens: 4848,
        cache_creation_tokens: 29335,
        cost_usd_micros: 6704,
    };
    let clone = Clone {
        id,
        handle: Handle::new(1),
        project: "app".to_owned(),
        account: "personal".to_owned(),
        mode: Mode::Headless,
        state: State::Working,
        reason: None,
        branch: "cloneyard/ct-0001".to_owned(),
        checkout: "/data/clones/x/checkout".to_owned(),
        unsandboxed: false,
        sessions: vec![Session {
            id: session,
            provider_id: "2f6c".to_owned(),
            usage,
        }],
        usage,
    };

    let json = serde_json::to_value(&clone).unwrap();

    let usage_json = json!({
        "input_tokens": 2,
        "output_tokens": 10,
        "cache_read_tokens": 4848,
        "cache_creation_tokens": 29335,
        "cost_usd_micros": 6704,
    });
    assert_eq!(
        json,
        json!({
            "id": id.to_string(),
            "handle": "CT-0001",
            "project": "app",
            "account": "personal",
            "mode": "headless",
            "state": "working",
            "reason": null,
            "branch": "cloneyard/ct-0001",
            "checkout": "/data/clones/x/checkout",
            "unsandboxed": false,
            "sessions": [{"id": session.to_string(), "provider_id": "2f6c", "usage": usage_json}],
            "usage": usage_json,
        })
    );
    assert_eq!(serde_json::from_value::<Clone>(json).unwrap(), clone);
}

#[test]
fn every_state_goes_on_the_wire_as_it_prints() {
    for state in [
        State::Working,
        State::Blocked,
        State::Idle,
        State::Done,
        State::Error,
        State::Stopped,
    ] {
        let json = serde_json::to_value(state).unwrap();

        assert_eq!(json, json!(state.to_string()));
        assert_eq!(serde_json::from_value::<State>(json).unwrap(), state);
    }
}

#[test]
fn hook_events_go_on_the_wire_with_their_time_session_hook_and_tool() {
    let id = Ulid::generate();
    let result = EventsResult {
        events: vec![HookEvent {
            id,
            at: 1_764_000_000_123,
            session: "2f6c".to_owned(),
            hook: "PreToolUse".to_owned(),
            tool: Some("Bash".to_owned()),
        }],
    };

    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(
        json,
        json!({"events": [{
            "id": id.to_string(),
            "at": 1_764_000_000_123_i64,
            "session": "2f6c",
            "hook": "PreToolUse",
            "tool": "Bash",
        }]})
    );
    assert_eq!(
        serde_json::from_value::<EventsResult>(json).unwrap(),
        result
    );
}

#[test]
fn usages_add_up_field_by_field() {
    let usage = |tokens: u64| Usage {
        input_tokens: tokens,
        output_tokens: tokens * 2,
        cache_read_tokens: tokens * 3,
        cache_creation_tokens: tokens * 4,
        cost_usd_micros: tokens * 5,
    };

    let total: Usage = [usage(1), usage(10)].into_iter().sum();

    assert_eq!(total, usage(11));
    assert_eq!([].into_iter().sum::<Usage>(), Usage::default());
}
