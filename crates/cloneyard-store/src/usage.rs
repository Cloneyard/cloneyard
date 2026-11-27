use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;
use cloneyard_api::usage::ToolUsage;
use ulid::Ulid;

use crate::store::Store;

/// One tool call with what the counts need of it.
struct Call {
    session: String,
    tool: String,
    command: Option<String>,
    path: Option<String>,
    checkout: String,
    failed: bool,
    denied: bool,
    result_bytes: Option<i64>,
}

/// The row of a call: its tool and the start of its command or its path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Row {
    tool: String,
    command: Option<String>,
    path: Option<String>,
}

#[derive(Default)]
struct Tally {
    calls: u64,
    sized: u64,
    result_bytes: u64,
    errors: u64,
    retries: u64,
    denials: u64,
}

impl Store {
    /// The tool calls of the clones of node `node_id` that have a record at or after `since`, in milliseconds since the Unix epoch, of project `project_id` alone when given, per tool and command or path; sorted by the bytes of their results in total, most first.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds a negative result size.
    pub async fn tool_usage(
        &self,
        node_id: Ulid,
        since: i64,
        project_id: Option<Ulid>,
    ) -> anyhow::Result<Vec<ToolUsage>> {
        self.call(move |connection| {
            // A record without a tool_use_id is a call of its own.
            let mut statement = connection.prepare(
                "WITH record AS ( \
                     SELECT clone_id, session_id, COALESCE(tool_use_id, 'result ' || seq) AS call, \
                     tool_name, at, success, result_size_bytes, 0 AS denied FROM tool_result \
                     WHERE at >= ?2 \
                     UNION ALL \
                     SELECT clone_id, session_id, COALESCE(tool_use_id, 'decision ' || seq), \
                     tool_name, at, NULL, NULL, decision = 'reject' FROM tool_decision \
                     WHERE at >= ?2 \
                 ), \
                 call AS ( \
                     SELECT clone_id, call, MIN(session_id) AS session_id, MIN(tool_name) AS tool_name, \
                     MIN(at) AS at, COALESCE(MAX(success = 0), 0) AS failed, \
                     MAX(result_size_bytes) AS result_bytes, MAX(denied) AS denied \
                     FROM record GROUP BY clone_id, call \
                 ) \
                 SELECT session.provider_id, COALESCE(hook.tool, call.tool_name), hook.command, \
                 hook.path, clone.checkout, call.failed, call.denied, call.result_bytes FROM call \
                 JOIN clone ON clone.id = call.clone_id \
                 JOIN session ON session.id = call.session_id \
                 LEFT JOIN hook_event AS hook ON hook.seq = ( \
                     SELECT seq FROM hook_event \
                     WHERE clone_id = call.clone_id AND tool_use_id = call.call \
                     ORDER BY seq LIMIT 1 \
                 ) \
                 WHERE clone.node_id = ?1 AND (?3 IS NULL OR clone.project_id = ?3) \
                 ORDER BY call.session_id, call.at, call.call",
            )?;
            let calls = statement
                .query_map(
                    (
                        node_id.to_string(),
                        since,
                        project_id.map(|id| id.to_string()),
                    ),
                    |row| {
                        Ok(Call {
                            session: row.get(0)?,
                            tool: row.get(1)?,
                            command: row.get(2)?,
                            path: row.get(3)?,
                            checkout: row.get(4)?,
                            failed: row.get(5)?,
                            denied: row.get(6)?,
                            result_bytes: row.get(7)?,
                        })
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            tally(calls)
        })
        .await
        .context("cannot count the tool calls")
    }
}

/// Counts `calls`, which come in the order they ran within each session.
fn tally(calls: Vec<Call>) -> anyhow::Result<Vec<ToolUsage>> {
    let mut rows: HashMap<Row, Tally> = HashMap::new();
    let mut failed_last: HashMap<(String, Row), bool> = HashMap::new();
    for call in calls {
        let row = Row {
            tool: call.tool,
            command: call.command.as_deref().and_then(command_start),
            path: call
                .path
                .as_deref()
                .map(|path| in_checkout(path, &call.checkout)),
        };
        let error = call.failed && !call.denied;
        let failed_before = failed_last.insert((call.session, row.clone()), error);
        let tally = rows.entry(row).or_default();
        tally.calls += 1;
        tally.errors += u64::from(error);
        tally.denials += u64::from(call.denied);
        tally.retries += u64::from(failed_before == Some(true));
        if let Some(bytes) = call.result_bytes {
            let bytes =
                u64::try_from(bytes).with_context(|| format!("invalid result size {bytes}"))?;
            tally.sized += 1;
            tally.result_bytes += bytes;
        }
    }
    let mut rows: Vec<(Row, Tally)> = rows.into_iter().collect();
    rows.sort_by(|(row, tally), (other_row, other)| {
        other
            .result_bytes
            .cmp(&tally.result_bytes)
            .then_with(|| row.cmp(other_row))
    });
    Ok(rows
        .into_iter()
        .map(|(row, tally)| ToolUsage {
            tool: row.tool,
            command: row.command,
            path: row.path,
            calls: tally.calls,
            average_result_tokens: (tally.sized > 0)
                .then(|| (tally.result_bytes + 2 * tally.sized) / (4 * tally.sized)),
            errors: tally.errors,
            retries: tally.retries,
            denials: tally.denials,
        })
        .collect())
}

/// The program of `command` and its first argument unless that is an option or a path, after leading `NAME=value` words and `cd <dir> &&`.
fn command_start(command: &str) -> Option<String> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let mut rest = words.as_slice();
    loop {
        while let [word, tail @ ..] = rest
            && is_assignment(word)
        {
            rest = tail;
        }
        match rest {
            ["cd", _, "&&", tail @ ..] => rest = tail,
            _ => break,
        }
    }
    let (program, arguments) = rest.split_first()?;
    Some(match arguments.first() {
        Some(argument) if is_subcommand(argument) => format!("{program} {argument}"),
        _ => (*program).to_owned(),
    })
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
            && name
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '_')
    })
}

fn is_subcommand(word: &str) -> bool {
    !word.starts_with('-')
        && word
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_')
}

/// `path` relative to `checkout` when it lies inside it.
fn in_checkout(path: &str, checkout: &str) -> String {
    match Path::new(path).strip_prefix(checkout) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => relative.display().to_string(),
        Err(_) => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use cloneyard_api::clone::Clone;
    use cloneyard_api::usage::ToolUsage;
    use tempfile::TempDir;

    use self::Outcome::{Denied, Failed, Ran};
    use super::{command_start, in_checkout};
    use crate::clone::tests::{Fixture, add, fixture};
    use crate::store::tests::open;
    use crate::{NewHookEvent, NewTelemetry, Store, TelemetryEvent, ToolDecision, ToolResult};

    const T: i64 = 1_764_000_000_000;

    /// What a tool call's hook says it is about.
    enum Target {
        Command(String),
        Path(String),
        Nothing,
        NoHook,
    }

    enum Outcome {
        Ran(Option<i64>),
        Failed(i64),
        Denied,
    }

    fn bash(command: &str) -> (&'static str, Target) {
        ("Bash", Target::Command(command.to_owned()))
    }

    fn read(path: &str) -> (&'static str, Target) {
        ("Read", Target::Path(path.to_owned()))
    }

    /// Adds a call that clone `clone` makes in session `session` at `T + offset`, with its `PreToolUse` hook unless the target says it has none.
    async fn call(
        store: &Store,
        clone: &Clone,
        (session, offset): (&str, i64),
        (tool, target): (&str, Target),
        outcome: Outcome,
    ) {
        let at = T + offset;
        let id = format!("toolu_{at}");
        let (command, path) = match &target {
            Target::Command(command) => (Some(command.clone()), None),
            Target::Path(path) => (None, Some(path.clone())),
            Target::Nothing | Target::NoHook => (None, None),
        };
        if !matches!(target, Target::NoHook) {
            store
                .add_hook_event(NewHookEvent {
                    clone_id: clone.id,
                    session: session.to_owned(),
                    hook: "PreToolUse".to_owned(),
                    tool: Some(tool.to_owned()),
                    tool_use_id: Some(id.clone()),
                    command,
                    path,
                    at,
                    file: id.clone(),
                    payload: "{}".to_owned(),
                    state: None,
                })
                .await
                .unwrap()
                .unwrap();
        }
        let record = |at, event| NewTelemetry {
            session: session.to_owned(),
            at,
            event,
        };
        let decision = |decision: &str| {
            TelemetryEvent::ToolDecision(ToolDecision {
                tool_use_id: Some(id.clone()),
                tool_name: tool.to_owned(),
                decision: decision.to_owned(),
                source: Some("config".to_owned()),
            })
        };
        let result = |success, bytes| {
            TelemetryEvent::ToolResult(ToolResult {
                tool_use_id: Some(id.clone()),
                tool_name: tool.to_owned(),
                success,
                duration_ms: Some(5),
                error_type: None,
                input_size_bytes: Some(20),
                result_size_bytes: bytes,
            })
        };
        let records = match outcome {
            Outcome::Ran(bytes) => vec![
                record(at, decision("accept")),
                record(at + 1, result(true, bytes)),
            ],
            Outcome::Failed(bytes) => vec![
                record(at, decision("accept")),
                record(at + 1, result(false, Some(bytes))),
            ],
            Outcome::Denied => vec![record(at, decision("reject"))],
        };
        assert_eq!(store.add_telemetry(clone.id, records).await.unwrap(), []);
    }

    fn row(
        (tool, command, path): (&str, Option<&str>, Option<&str>),
        calls: u64,
        average_result_tokens: Option<u64>,
        [errors, retries, denials]: [u64; 3],
    ) -> ToolUsage {
        ToolUsage {
            tool: tool.to_owned(),
            command: command.map(str::to_owned),
            path: path.map(str::to_owned),
            calls,
            average_result_tokens,
            errors,
            retries,
            denials,
        }
    }

    #[tokio::test]
    async fn counts_the_calls_per_tool_and_target_as_by_hand() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let first = add(&store, &fixture, "a1").await;
        let second = add(&store, &fixture, "b1").await;
        let readme = [&first, &second].map(|clone| format!("{}/README.md", clone.checkout));
        let cd_and_test = format!("cd {} && A=1 cargo test -p api", first.checkout);
        for (clone, at, tool, outcome) in [
            (&first, ("a1", 0), bash("gh api repos/a/b"), Failed(400)),
            (&first, ("a1", 10), read(&readme[0]), Ran(Some(4000))),
            (
                &first,
                ("a1", 20),
                bash("gh api repos/a/b --jq .x"),
                Ran(Some(2000)),
            ),
            (&first, ("a1", 30), bash("rm -rf target"), Denied),
            (&first, ("a1", 40), bash("rm -rf target"), Denied),
            (&first, ("a1", 50), bash(&cd_and_test), Failed(1000)),
            (
                &first,
                ("a2", 60),
                bash("cargo test --locked"),
                Ran(Some(3000)),
            ),
            (
                &first,
                ("a2", 70),
                ("TodoWrite", Target::Nothing),
                Ran(None),
            ),
            (&first, ("a2", 80), ("Bash", Target::NoHook), Ran(Some(100))),
            (&second, ("b1", 90), read(&readme[1]), Ran(Some(2000))),
            (&second, ("b1", 100), read("/home/me/notes.md"), Failed(80)),
            (&second, ("b1", 110), bash("cargo test"), Failed(500)),
            (&second, ("b1", 120), bash("cargo build"), Ran(Some(10))),
            (&second, ("b1", 130), bash("cargo test"), Ran(Some(700))),
        ] {
            call(&store, clone, at, tool, outcome).await;
        }

        let counted = store.tool_usage(fixture.node, T, None).await.unwrap();

        assert_eq!(
            counted,
            [
                row(("Read", None, Some("README.md")), 2, Some(750), [0, 0, 0]),
                row(("Bash", Some("cargo test"), None), 4, Some(325), [2, 1, 0]),
                row(("Bash", Some("gh api"), None), 2, Some(300), [1, 1, 0]),
                row(("Bash", None, None), 1, Some(25), [0, 0, 0]),
                row(
                    ("Read", None, Some("/home/me/notes.md")),
                    1,
                    Some(20),
                    [1, 0, 0]
                ),
                row(("Bash", Some("cargo build"), None), 1, Some(3), [0, 0, 0]),
                row(("Bash", Some("rm"), None), 2, None, [0, 0, 2]),
                row(("TodoWrite", None, None), 1, None, [0, 0, 0]),
            ]
        );
    }

    #[tokio::test]
    async fn leaves_out_the_calls_before_the_period_and_those_of_other_projects() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let app = fixture(&store).await;
        let web = store
            .add_project(app.node, "web", "web", "/code/web")
            .await
            .unwrap()
            .unwrap();
        let web = Fixture {
            project: web.id,
            repo: web.repos[0].id,
            ..app
        };
        let old = add(&store, &app, "app").await;
        let other = add(&store, &web, "web").await;
        call(&store, &old, ("app", -2), bash("cargo test"), Ran(Some(40))).await;
        call(&store, &old, ("app", 0), bash("cargo test"), Ran(Some(80))).await;
        call(
            &store,
            &other,
            ("web", 0),
            bash("cargo test"),
            Ran(Some(400)),
        )
        .await;

        let in_app = store
            .tool_usage(app.node, T, Some(app.project))
            .await
            .unwrap();
        let everywhere = store.tool_usage(app.node, T - 2, None).await.unwrap();

        let cargo_test = ("Bash", Some("cargo test"), None);
        assert_eq!(in_app, [row(cargo_test, 1, Some(20), [0, 0, 0])]);
        assert_eq!(everywhere, [row(cargo_test, 3, Some(43), [0, 0, 0])]);
    }

    #[tokio::test]
    async fn a_negative_result_size_in_the_database_is_named() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "s").await;
        call(&store, &clone, ("s", 0), bash("ls"), Ran(Some(-5))).await;

        let error = store.tool_usage(fixture.node, T, None).await.unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "cannot count the tool calls: invalid result size -5: number too small to fit in target type"
        );
    }

    #[test]
    fn a_command_starts_with_its_program_and_a_first_argument_that_is_no_option_or_path() {
        let starts: Vec<Option<String>> = [
            "gh api repos/a/b --jq .x",
            "git status",
            "ls -la",
            "cat README.md",
            "./scripts/check-licence-direction",
            "UPDATE_SCHEMA=1 cargo test -p cloneyard-api",
            "cd /c && npm run build",
            "cd /c && A=1 _B=2 make all",
            "cd /c",
            "echo \"two words\"",
            "1A=1 ls",
            "A=1",
            "",
        ]
        .into_iter()
        .map(command_start)
        .collect();

        let some = |start: &str| Some(start.to_owned());
        assert_eq!(
            starts,
            [
                some("gh api"),
                some("git status"),
                some("ls"),
                some("cat"),
                some("./scripts/check-licence-direction"),
                some("cargo test"),
                some("npm run"),
                some("make all"),
                some("cd"),
                some("echo"),
                some("1A=1 ls"),
                None,
                None,
            ]
        );
    }

    #[test]
    fn a_path_in_the_checkout_is_relative_to_it_and_any_other_stays_as_it_is() {
        let paths = [
            "/c/x/README.md",
            "/c/x/src/lib.rs",
            "/c/x",
            "/c/xy/a.rs",
            "notes.md",
        ]
        .map(|path| in_checkout(path, "/c/x"));

        assert_eq!(
            paths,
            ["README.md", "src/lib.rs", ".", "/c/xy/a.rs", "notes.md"]
        );
    }
}
