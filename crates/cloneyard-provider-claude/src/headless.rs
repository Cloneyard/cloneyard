//! A headless `claude`: a prompt in and stream-json out, one JSON object per line on stdin and on stdout.

use std::path::Path;

use serde_json::{Value, json};
use tokio::process::Command;
use uuid::Uuid;

/// `claude -p` for the account in `dir`, on session `session_id`, which denies at once any tool that would ask for permission.
#[must_use]
pub fn command(dir: &Path, session_id: Uuid) -> Command {
    let mut claude = crate::command(dir);
    claude
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--session-id",
        ])
        .arg(session_id.to_string())
        .args(["--permission-prompts", "none"]);
    claude
}

/// The line on stdin that sends `text` as the user's next message.
#[must_use]
pub fn user_message(text: &str) -> String {
    let message = json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    });
    format!("{message}\n")
}

/// A line on stdout that tells something about the clone's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `system/init`: the session has started, with the provider's id of it.
    Init { session_id: String },
    /// `result`: the turn has ended.
    Result(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    Error { reason: String },
}

/// The event on a line of stdout, or `None` for any other line, JSON or not.
#[must_use]
pub fn parse(line: &str) -> Option<Event> {
    let event: Value = serde_json::from_str(line).ok()?;
    match (text(&event, "type")?, text(&event, "subtype")) {
        ("system", Some("init")) => Some(Event::Init {
            session_id: text(&event, "session_id")?.to_owned(),
        }),
        ("result", _) => Some(Event::Result(outcome(&event))),
        _ => None,
    }
}

// An error arrives as `is_error: true` with `subtype: success`, so the subtype says nothing.
fn outcome(result: &Value) -> Outcome {
    if result.get("is_error").and_then(Value::as_bool) != Some(true) {
        return Outcome::Done;
    }
    let terminal_reason = text(result, "terminal_reason").filter(|reason| !reason.is_empty());
    let first_line = text(result, "result")
        .and_then(|result| result.lines().find(|line| !line.trim().is_empty()))
        .map(str::trim);
    let reason = match (terminal_reason, first_line) {
        (Some(reason), Some(line)) => format!("{reason}: {line}"),
        (Some(reason), None) => reason.to_owned(),
        (None, Some(line)) => line.to_owned(),
        (None, None) => "claude ended its turn with an error".to_owned(),
    };
    Outcome::Error { reason }
}

fn text<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::{Event, Outcome, command, parse, user_message};

    #[test]
    fn the_command_reads_and_writes_stream_json_on_the_given_session_without_permission_prompts() {
        let session_id = Uuid::new_v4();

        let claude = command(Path::new("/data/accounts/personal/claude"), session_id);

        let args: Vec<String> = claude
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--session-id",
                &session_id.to_string(),
                "--permission-prompts",
                "none",
            ]
        );
    }

    #[test]
    fn a_user_message_is_one_line_of_json_with_the_text() {
        let line = user_message("Say \"pong\"\nthen stop");

        let (json, rest) = line.split_once('\n').unwrap();
        assert_eq!(rest, "");
        assert_eq!(
            serde_json::from_str::<Value>(json).unwrap(),
            json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Say \"pong\"\nthen stop"}]
                }
            })
        );
    }

    #[test]
    fn init_gives_the_session_id() {
        let line = json!({"type": "system", "subtype": "init", "session_id": "2f6c", "cwd": "/w"});

        assert_eq!(
            parse(&line.to_string()),
            Some(Event::Init {
                session_id: "2f6c".to_owned()
            })
        );
    }

    #[test]
    fn a_result_without_is_error_is_done() {
        for result in [
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "pong"}),
            json!({"type": "result", "subtype": "success", "result": "pong"}),
        ] {
            assert_eq!(
                parse(&result.to_string()),
                Some(Event::Result(Outcome::Done))
            );
        }
    }

    #[test]
    fn a_result_with_is_error_is_an_error_even_with_subtype_success() {
        let result = json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "terminal_reason": "api_error",
            "result": "\nFailed to refresh OAuth token\nRetry later",
        });

        assert_eq!(
            parse(&result.to_string()),
            Some(Event::Result(Outcome::Error {
                reason: "api_error: Failed to refresh OAuth token".to_owned()
            }))
        );
    }

    #[test]
    fn the_reason_of_an_error_is_whichever_of_terminal_reason_and_result_it_has() {
        let reasons: Vec<Option<Event>> = [
            json!({"type": "result", "is_error": true, "terminal_reason": "max_turns"}),
            json!({"type": "result", "is_error": true, "result": "Prompt is too long"}),
            json!({"type": "result", "is_error": true, "terminal_reason": "", "result": " "}),
        ]
        .iter()
        .map(|result| parse(&result.to_string()))
        .collect();

        let error = |reason: &str| {
            Some(Event::Result(Outcome::Error {
                reason: reason.to_owned(),
            }))
        };
        assert_eq!(
            reasons,
            [
                error("max_turns"),
                error("Prompt is too long"),
                error("claude ended its turn with an error"),
            ]
        );
    }

    #[test]
    fn other_lines_are_no_event() {
        for line in [
            json!({"type": "assistant", "message": {}}).to_string(),
            json!({"type": "system", "subtype": "api_retry", "attempt": 1}).to_string(),
            json!({"type": "system", "subtype": "init"}).to_string(),
            json!(["result"]).to_string(),
            "not json".to_owned(),
            String::new(),
        ] {
            assert_eq!(parse(&line), None, "{line}");
        }
    }
}
