//! Claude Code's hooks: the settings that make a session run `cy hook` for each of them, and what Cloneyard reads from the JSON a hook gets on stdin.

use serde_json::{Map, Value, json};

/// The hooks Cloneyard listens to.
pub const EVENTS: [&str; 11] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Notification",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "Stop",
    "SessionEnd",
];

const TOOL_EVENTS: [&str; 3] = ["PreToolUse", "PostToolUse", "PostToolUseFailure"];

/// How long Claude Code waits for a hook, in seconds; `cy hook` only writes a file.
const TIMEOUT: u64 = 10;

/// The `hooks` of the settings: for each of `EVENTS`, a hook that runs `program hook <event>` and that Claude Code waits for, so one session's hooks run in order.
pub(crate) fn settings(program: &str) -> Value {
    let program = quote(program);
    let hooks: Map<String, Value> = EVENTS
        .iter()
        .map(|event| {
            let mut entry = json!({
                "hooks": [{
                    "type": "command",
                    "command": format!("{program} hook {event}"),
                    "timeout": TIMEOUT,
                }],
            });
            if TOOL_EVENTS.contains(event) {
                entry["matcher"] = json!("*");
            }
            ((*event).to_owned(), json!([entry]))
        })
        .collect();
    Value::Object(hooks)
}

/// `text` as one word for `sh -c`, which runs a hook's command.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// What the JSON of a hook tells Cloneyard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    /// The provider's id of the session the hook ran in.
    pub session_id: String,
    /// The tool of a tool hook.
    pub tool: Option<String>,
    /// The id of the tool call of a tool hook, which its telemetry also carries.
    pub tool_use_id: Option<String>,
    /// What the tool call is about, for the tools that say.
    pub target: Option<Target>,
    /// The state of the clone that the hook tells, if any.
    pub state: Option<State>,
    /// The JSON without `tool_response`, which can hold a whole file.
    pub payload: String,
}

/// What a tool call is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The command that `Bash` runs.
    Command(String),
    /// The file that `Read`, `Write`, `Edit` or `NotebookEdit` reads or changes.
    Path(String),
}

impl Target {
    fn of(tool: &str, input: &Map<String, Value>) -> Option<Self> {
        let field = |key| text(input, key).map(str::to_owned);
        match tool {
            "Bash" => field("command").map(Self::Command),
            "Read" | "Write" | "Edit" => field("file_path").map(Self::Path),
            "NotebookEdit" => field("notebook_path").map(Self::Path),
            _ => None,
        }
    }
}

/// A state of a running clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Working,
    /// Waits for a permission.
    Blocked,
    /// Ended its turn and waits for a message.
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Invalid {
    #[error("unknown hook {0:?}")]
    Event(String),
    #[error("not a JSON object: {0}")]
    Json(String),
    #[error("no session_id")]
    Session,
}

/// Reads the JSON that hook `event` got on stdin.
///
/// # Errors
///
/// When `event` isn't one of `EVENTS`, or the JSON isn't an object with a `session_id`.
pub fn parse(event: &str, json: &[u8]) -> Result<Hook, Invalid> {
    if !EVENTS.contains(&event) {
        return Err(Invalid::Event(event.to_owned()));
    }
    let mut payload: Map<String, Value> =
        serde_json::from_slice(json).map_err(|error| Invalid::Json(error.to_string()))?;
    let session_id = text(&payload, "session_id")
        .filter(|id| !id.is_empty())
        .ok_or(Invalid::Session)?
        .to_owned();
    let tool = text(&payload, "tool_name").map(str::to_owned);
    let tool_use_id = text(&payload, "tool_use_id").map(str::to_owned);
    let target = tool
        .as_deref()
        .zip(payload.get("tool_input").and_then(Value::as_object))
        .and_then(|(tool, input)| Target::of(tool, input));
    let state = match event {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
            Some(State::Working)
        }
        "Notification" if text(&payload, "notification_type") == Some("permission_prompt") => {
            Some(State::Blocked)
        }
        "Stop" => Some(State::Idle),
        _ => None,
    };
    payload.remove("tool_response");
    Ok(Hook {
        session_id,
        tool,
        tool_use_id,
        target,
        state,
        payload: Value::Object(payload).to_string(),
    })
}

fn text<'a>(payload: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{EVENTS, Hook, Invalid, State, Target, parse, settings};

    fn hook(event: &str, payload: &Value) -> Result<Hook, Invalid> {
        parse(event, payload.to_string().as_bytes())
    }

    #[test]
    fn each_hook_runs_the_quoted_program_with_its_event_and_tool_hooks_match_every_tool() {
        let hooks = settings("/opt/it's here/cy");

        let events: Vec<&String> = hooks.as_object().unwrap().keys().collect();
        let mut expected = EVENTS.to_vec();
        expected.sort_unstable();
        assert_eq!(events, expected);
        assert_eq!(
            hooks["PostToolUseFailure"],
            json!([{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": r"'/opt/it'\''s here/cy' hook PostToolUseFailure",
                    "timeout": 10,
                }],
            }])
        );
        assert_eq!(
            hooks["Stop"],
            json!([{
                "hooks": [{
                    "type": "command",
                    "command": r"'/opt/it'\''s here/cy' hook Stop",
                    "timeout": 10,
                }],
            }])
        );
    }

    #[test]
    fn a_tool_hook_gives_its_session_and_tool_and_tells_working_without_the_tool_response() {
        let payload = json!({
            "session_id": "2f6c",
            "hook_event_name": "PostToolUse",
            "tool_name": "Read",
            "tool_input": {"file_path": "README.md"},
            "tool_response": {"content": "a whole file"},
        });

        let parsed = hook("PostToolUse", &payload).unwrap();

        assert_eq!(
            (
                parsed.session_id.as_str(),
                parsed.tool.as_deref(),
                parsed.state
            ),
            ("2f6c", Some("Read"), Some(State::Working))
        );
        assert_eq!(
            serde_json::from_str::<Value>(&parsed.payload).unwrap(),
            json!({
                "session_id": "2f6c",
                "hook_event_name": "PostToolUse",
                "tool_name": "Read",
                "tool_input": {"file_path": "README.md"},
            })
        );
    }

    #[test]
    fn a_tool_hook_gives_its_call_id_and_the_command_or_path_that_its_tool_names() {
        let calls: Vec<(Option<String>, Option<Target>)> = [
            (
                "Bash",
                json!({"command": "gh api repos", "description": "List"}),
            ),
            ("Read", json!({"file_path": "/c/README.md", "limit": 10})),
            (
                "Write",
                json!({"file_path": "/c/new.rs", "content": "fn main() {}"}),
            ),
            ("Edit", json!({"file_path": "/c/lib.rs", "old_string": "a"})),
            ("NotebookEdit", json!({"notebook_path": "/c/a.ipynb"})),
            ("Grep", json!({"pattern": "fn main", "path": "/c"})),
            ("Bash", json!({"description": "no command"})),
            ("Read", json!("not an object")),
        ]
        .into_iter()
        .map(|(tool, input)| {
            let payload = json!({
                "session_id": "s",
                "tool_name": tool,
                "tool_use_id": "toolu_01",
                "tool_input": input,
            });
            let parsed = hook("PreToolUse", &payload).unwrap();
            (parsed.tool_use_id, parsed.target)
        })
        .collect();

        let id = Some("toolu_01".to_owned());
        let command = |text: &str| Some(Target::Command(text.to_owned()));
        let path = |text: &str| Some(Target::Path(text.to_owned()));
        assert_eq!(
            calls,
            [
                (id.clone(), command("gh api repos")),
                (id.clone(), path("/c/README.md")),
                (id.clone(), path("/c/new.rs")),
                (id.clone(), path("/c/lib.rs")),
                (id.clone(), path("/c/a.ipynb")),
                (id.clone(), None),
                (id.clone(), None),
                (id, None),
            ]
        );
    }

    #[test]
    fn a_hook_without_a_tool_has_no_call_id_or_target() {
        let parsed = hook("Stop", &json!({"session_id": "s"})).unwrap();

        assert_eq!((parsed.tool_use_id, parsed.target), (None, None));
    }

    #[test]
    fn a_permission_prompt_tells_blocked_stop_idle_and_a_prompt_working() {
        let states: Vec<Option<State>> = [
            (
                "Notification",
                json!({"session_id": "s", "notification_type": "permission_prompt"}),
            ),
            (
                "Notification",
                json!({"session_id": "s", "notification_type": "idle_prompt"}),
            ),
            ("Stop", json!({"session_id": "s"})),
            (
                "UserPromptSubmit",
                json!({"session_id": "s", "prompt": "Hi"}),
            ),
            (
                "PreToolUse",
                json!({"session_id": "s", "tool_name": "Bash"}),
            ),
            (
                "PostToolUseFailure",
                json!({"session_id": "s", "tool_name": "Bash"}),
            ),
            (
                "SessionStart",
                json!({"session_id": "s", "source": "startup"}),
            ),
            ("SubagentStop", json!({"session_id": "s"})),
            ("SessionEnd", json!({"session_id": "s", "reason": "other"})),
        ]
        .iter()
        .map(|(event, payload)| hook(event, payload).unwrap().state)
        .collect();

        assert_eq!(
            states,
            [
                Some(State::Blocked),
                None,
                Some(State::Idle),
                Some(State::Working),
                Some(State::Working),
                Some(State::Working),
                None,
                None,
                None,
            ]
        );
    }

    #[test]
    fn an_unknown_hook_json_that_is_no_object_and_a_missing_session_are_invalid() {
        let errors: Vec<String> = [
            parse("PostCompact", br#"{"session_id": "s"}"#),
            parse("Stop", b"not json"),
            parse("Stop", br#"["session_id"]"#),
            parse("Stop", br#"{"session_id": ""}"#),
            parse("Stop", br#"{"session_id": 42}"#),
        ]
        .into_iter()
        .map(|parsed| parsed.unwrap_err().to_string())
        .collect();

        assert_eq!(
            errors,
            [
                "unknown hook \"PostCompact\"",
                "not a JSON object: expected ident at line 1 column 2",
                "not a JSON object: invalid type: sequence, expected a map at line 1 column 0",
                "no session_id",
                "no session_id",
            ]
        );
    }
}
