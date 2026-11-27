use anyhow::Context as _;
use serde_json::{Value, json};
use ulid::Ulid;

use crate::clone::session;
use crate::hook::Refused;
use crate::store::Store;

/// An event of a clone's session, from its provider's telemetry, to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTelemetry {
    /// The provider's id of the session it happened in.
    pub session: String,
    /// When it happened, in milliseconds since the Unix epoch.
    pub at: i64,
    pub event: TelemetryEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelemetryEvent {
    ApiRequest(ApiRequest),
    ApiError(ApiError),
    ToolResult(ToolResult),
    ToolDecision(ToolDecision),
}

/// A request to the model and the tokens it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRequest {
    pub request_id: Option<String>,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cost_usd_micros: i64,
    pub duration_ms: Option<i64>,
}

/// A request to the model that still failed after its retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub request_id: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
    pub status_code: Option<i64>,
    pub attempt: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// A tool call that ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: Option<i64>,
    pub error_type: Option<String>,
    pub input_size_bytes: Option<i64>,
    pub result_size_bytes: Option<i64>,
}

/// Whether a tool call may run, and what decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDecision {
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    pub decision: String,
    pub source: Option<String>,
}

/// The table of an event, the column and value of the provider's id that make it unique in a clone, and its other fields.
struct Row {
    table: &'static str,
    key: (&'static str, Option<String>),
    fields: Vec<(&'static str, Value)>,
}

impl TelemetryEvent {
    fn row(self) -> Row {
        match self {
            Self::ApiRequest(request) => Row {
                table: "api_request",
                key: ("request_id", request.request_id),
                fields: vec![
                    ("model", json!(request.model)),
                    ("input_tokens", json!(request.input_tokens)),
                    ("output_tokens", json!(request.output_tokens)),
                    ("cache_read_tokens", json!(request.cache_read_tokens)),
                    (
                        "cache_creation_tokens",
                        json!(request.cache_creation_tokens),
                    ),
                    ("cost_usd_micros", json!(request.cost_usd_micros)),
                    ("duration_ms", json!(request.duration_ms)),
                ],
            },
            Self::ApiError(error) => Row {
                table: "api_error",
                key: ("request_id", error.request_id),
                fields: vec![
                    ("model", json!(error.model)),
                    ("error", json!(error.error)),
                    ("status_code", json!(error.status_code)),
                    ("attempt", json!(error.attempt)),
                    ("duration_ms", json!(error.duration_ms)),
                ],
            },
            Self::ToolResult(result) => Row {
                table: "tool_result",
                key: ("tool_use_id", result.tool_use_id),
                fields: vec![
                    ("tool_name", json!(result.tool_name)),
                    ("success", json!(result.success)),
                    ("duration_ms", json!(result.duration_ms)),
                    ("error_type", json!(result.error_type)),
                    ("input_size_bytes", json!(result.input_size_bytes)),
                    ("result_size_bytes", json!(result.result_size_bytes)),
                ],
            },
            Self::ToolDecision(decision) => Row {
                table: "tool_decision",
                key: ("tool_use_id", decision.tool_use_id),
                fields: vec![
                    ("tool_name", json!(decision.tool_name)),
                    ("decision", json!(decision.decision)),
                    ("source", json!(decision.source)),
                ],
            },
        }
    }
}

impl Store {
    /// Adds the telemetry of clone `clone_id` from one export in one transaction, with each session that is new, and gives the records it refused; a record whose request or tool call the clone has already adds nothing.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn add_telemetry(
        &self,
        clone_id: Ulid,
        records: Vec<NewTelemetry>,
    ) -> anyhow::Result<Vec<Refused>> {
        self.write(move |changes| {
            let known: bool = changes.query_row(
                "SELECT EXISTS (SELECT 1 FROM clone WHERE id = ?1)",
                [clone_id.to_string()],
                |row| row.get(0),
            )?;
            if !known {
                return Ok(vec![Refused::UnknownClone(clone_id)]);
            }
            let mut refused = Vec::new();
            for NewTelemetry {
                session: provider_id,
                at,
                event,
            } in records
            {
                let session_id = match session(changes, clone_id, &provider_id)? {
                    Ok(id) => id,
                    Err(owner) => {
                        refused.push(Refused::ForeignSession {
                            session: provider_id,
                            owner,
                        });
                        continue;
                    }
                };
                let Row {
                    table,
                    key: (column, key),
                    fields,
                } = event.row();
                if let Some(key) = &key {
                    let taken: bool = changes.query_row(
                        &format!(
                            "SELECT EXISTS (SELECT 1 FROM {table} WHERE clone_id = ?1 AND {column} = ?2)"
                        ),
                        (clone_id.to_string(), key),
                        |row| row.get(0),
                    )?;
                    if taken {
                        continue;
                    }
                }
                let mut row = vec![
                    ("id", json!(Ulid::generate())),
                    ("clone_id", json!(clone_id)),
                    ("session_id", json!(session_id)),
                    ("at", json!(at)),
                    (column, json!(key)),
                ];
                row.extend(fields);
                changes.append(table, &row)?;
            }
            Ok(refused)
        })
        .await
        .with_context(|| format!("cannot add the telemetry of clone {clone_id}"))
    }
}

#[cfg(test)]
mod tests {
    use cloneyard_api::clone::{Clone, Usage};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::{ApiError, ApiRequest, NewTelemetry, TelemetryEvent, ToolDecision, ToolResult};
    use crate::Store;
    use crate::clone::tests::{add, fixture};
    use crate::hook::Refused;
    use crate::store::tests::{change_log, open, sql};

    fn request(session: &str, request_id: Option<&str>, input_tokens: i64) -> NewTelemetry {
        NewTelemetry {
            session: session.to_owned(),
            at: 1_764_000_000_000,
            event: TelemetryEvent::ApiRequest(ApiRequest {
                request_id: request_id.map(str::to_owned),
                model: Some("claude-haiku-5-5".to_owned()),
                input_tokens,
                output_tokens: 10,
                cache_read_tokens: 4848,
                cache_creation_tokens: 29335,
                cost_usd_micros: 6704,
                duration_ms: Some(1014),
            }),
        }
    }

    fn tool(session: &str, tool_use_id: &str) -> [NewTelemetry; 2] {
        let id = Some(tool_use_id.to_owned());
        [
            NewTelemetry {
                session: session.to_owned(),
                at: 1_764_000_000_001,
                event: TelemetryEvent::ToolDecision(ToolDecision {
                    tool_use_id: id.clone(),
                    tool_name: "Bash".to_owned(),
                    decision: "accept".to_owned(),
                    source: Some("config".to_owned()),
                }),
            },
            NewTelemetry {
                session: session.to_owned(),
                at: 1_764_000_000_002,
                event: TelemetryEvent::ToolResult(ToolResult {
                    tool_use_id: id,
                    tool_name: "Bash".to_owned(),
                    success: true,
                    duration_ms: Some(603),
                    error_type: None,
                    input_size_bytes: Some(74),
                    result_size_bytes: Some(11),
                }),
            },
        ]
    }

    fn api_error(session: &str) -> NewTelemetry {
        NewTelemetry {
            session: session.to_owned(),
            at: 1_764_000_000_003,
            event: TelemetryEvent::ApiError(ApiError {
                request_id: None,
                model: None,
                error: Some("Overloaded".to_owned()),
                status_code: Some(529),
                attempt: Some(3),
                duration_ms: None,
            }),
        }
    }

    async fn with_clone(dir: &TempDir) -> (Store, Clone) {
        let store = open(dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;
        (store, clone)
    }

    fn rows(dir: &TempDir, query: &str) -> Vec<Value> {
        let connection = sql(dir);
        let mut statement = connection.prepare(query).unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| {
                (0..count)
                    .map(|index| {
                        Ok(match row.get_ref(index)? {
                            rusqlite::types::ValueRef::Null => Value::Null,
                            rusqlite::types::ValueRef::Integer(number) => json!(number),
                            rusqlite::types::ValueRef::Text(text) => {
                                json!(String::from_utf8_lossy(text))
                            }
                            other => panic!("unexpected {other:?}"),
                        })
                    })
                    .collect::<rusqlite::Result<Value>>()
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[tokio::test]
    async fn adds_each_event_to_its_table_with_its_clone_and_session() {
        let dir = TempDir::new().unwrap();
        let (store, clone) = with_clone(&dir).await;
        let [decision, result] = tool("first", "toolu_01");

        let refused = store
            .add_telemetry(
                clone.id,
                vec![
                    request("first", Some("req_1"), 2),
                    decision,
                    result,
                    api_error("first"),
                ],
            )
            .await
            .unwrap();

        assert_eq!(refused, []);
        let session = clone.sessions[0].id.to_string();
        let clone_id = clone.id.to_string();
        assert_eq!(
            rows(
                &dir,
                "SELECT clone_id, session_id, at, request_id, model, input_tokens, output_tokens, \
                 cache_read_tokens, cache_creation_tokens, cost_usd_micros, duration_ms \
                 FROM api_request"
            ),
            [json!([
                clone_id,
                session,
                1_764_000_000_000_i64,
                "req_1",
                "claude-haiku-5-5",
                2,
                10,
                4848,
                29335,
                6704,
                1014
            ])]
        );
        assert_eq!(
            rows(
                &dir,
                "SELECT tool_use_id, tool_name, decision, source FROM tool_decision"
            ),
            [json!(["toolu_01", "Bash", "accept", "config"])]
        );
        assert_eq!(
            rows(
                &dir,
                "SELECT tool_use_id, tool_name, success, duration_ms, error_type, \
                 input_size_bytes, result_size_bytes FROM tool_result"
            ),
            [json!(["toolu_01", "Bash", 1, 603, null, 74, 11])]
        );
        assert_eq!(
            rows(
                &dir,
                "SELECT request_id, error, status_code, attempt FROM api_error"
            ),
            [json!([null, "Overloaded", 529, 3])]
        );
    }

    #[tokio::test]
    async fn a_request_or_tool_call_sent_again_adds_nothing_and_one_without_an_id_adds_each_time() {
        let dir = TempDir::new().unwrap();
        let (store, clone) = with_clone(&dir).await;

        for _ in 0..2 {
            let [decision, result] = tool("first", "toolu_01");
            let export = vec![
                request("first", Some("req_1"), 2),
                request("first", Some("req_1"), 2),
                decision,
                result,
                api_error("first"),
            ];
            store.add_telemetry(clone.id, export).await.unwrap();
        }

        let counts = rows(
            &dir,
            "SELECT (SELECT count(*) FROM api_request), (SELECT count(*) FROM tool_decision), \
             (SELECT count(*) FROM tool_result), (SELECT count(*) FROM api_error)",
        );
        assert_eq!(counts, [json!([1, 1, 1, 2])]);
    }

    #[tokio::test]
    async fn a_new_session_joins_the_clone_and_another_clones_session_is_refused_alone() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;
        let other = add(&store, &fixture, "other").await;

        let refused = store
            .add_telemetry(
                clone.id,
                vec![
                    request("other", Some("req_1"), 1),
                    request("second", Some("req_2"), 2),
                ],
            )
            .await
            .unwrap();

        assert_eq!(
            refused,
            [Refused::ForeignSession {
                session: "other".to_owned(),
                owner: other.id
            }]
        );
        let sessions: Vec<String> = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap()
            .sessions
            .into_iter()
            .map(|session| session.provider_id)
            .collect();
        assert_eq!(sessions, ["first", "second"]);
        assert_eq!(
            rows(&dir, "SELECT request_id FROM api_request"),
            [json!(["req_2"])]
        );
    }

    #[tokio::test]
    async fn the_records_of_an_unknown_clone_are_refused_and_nothing_is_written() {
        let dir = TempDir::new().unwrap();
        let (store, _) = with_clone(&dir).await;
        let unknown = Ulid::generate();
        let logged = change_log(&dir).len();

        let refused = store
            .add_telemetry(unknown, vec![request("third", Some("req_1"), 1)])
            .await
            .unwrap();

        assert_eq!(refused, [Refused::UnknownClone(unknown)]);
        assert_eq!(
            rows(&dir, "SELECT id FROM api_request"),
            Vec::<Value>::new()
        );
        assert_eq!(change_log(&dir).len(), logged);
    }

    #[tokio::test]
    async fn a_session_has_the_usage_of_its_requests_and_a_clone_the_sum_of_its_sessions() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;
        let idle = add(&store, &fixture, "idle").await;
        store
            .add_telemetry(
                clone.id,
                vec![
                    request("first", Some("req_1"), 2),
                    request("first", Some("req_2"), 3),
                    request("second", Some("req_3"), 5),
                    api_error("first"),
                ],
            )
            .await
            .unwrap();

        let shown = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap();
        let listed = store.clones(fixture.node).await.unwrap();

        let usage = |requests: u64, input_tokens| Usage {
            input_tokens,
            output_tokens: 10 * requests,
            cache_read_tokens: 4848 * requests,
            cache_creation_tokens: 29335 * requests,
            cost_usd_micros: 6704 * requests,
        };
        let sessions: Vec<Usage> = shown.sessions.iter().map(|session| session.usage).collect();
        assert_eq!(sessions, [usage(2, 5), usage(1, 5)]);
        assert_eq!(shown.usage, usage(3, 10));
        assert_eq!(listed[0], shown);
        assert_eq!(listed[1].handle, idle.handle);
        assert_eq!(listed[1].usage, Usage::default());
        assert_eq!(listed[1].sessions[0].usage, Usage::default());
    }

    #[tokio::test]
    async fn a_negative_sum_in_the_database_is_named() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;
        store
            .add_telemetry(clone.id, vec![request("first", Some("req_1"), 2)])
            .await
            .unwrap();
        sql(&dir)
            .execute("UPDATE api_request SET input_tokens = -5", [])
            .unwrap();

        let error = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "cannot read clone CT-0001: invalid usage -5: number too small to fit in target type"
        );
    }

    #[tokio::test]
    async fn the_change_log_takes_a_new_session_but_not_the_records() {
        let dir = TempDir::new().unwrap();
        let (store, clone) = with_clone(&dir).await;
        let before = change_log(&dir).len();

        store
            .add_telemetry(
                clone.id,
                vec![
                    request("first", Some("req_1"), 1),
                    request("second", Some("req_2"), 2),
                ],
            )
            .await
            .unwrap();

        let logged: Vec<(String, Value)> = change_log(&dir)
            .into_iter()
            .skip(before)
            .map(|logged| (logged.entity, logged.fields))
            .collect();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].0, "session");
        assert_eq!(logged[0].1["provider_id"], json!("second"));
    }
}
