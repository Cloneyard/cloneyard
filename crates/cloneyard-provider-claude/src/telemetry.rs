//! Claude Code's OpenTelemetry: the variables that send a session's events to Cloneyard's receiver, and what Cloneyard keeps of an export of them.

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;

use serde::Deserialize;
use tokio::process::Command;

/// Sends `claude`'s OpenTelemetry events, and no metrics or traces, as OTLP/HTTP JSON to the receiver on port `port` of `127.0.0.1`, with `token` as the bearer token that tells the receiver which clone sent them.
pub fn send_events(claude: &mut Command, port: u16, token: &str) {
    claude
        .env("CLAUDE_CODE_ENABLE_TELEMETRY", "1")
        .env("OTEL_LOGS_EXPORTER", "otlp")
        .env("OTEL_METRICS_EXPORTER", "none")
        .env("OTEL_TRACES_EXPORTER", "none")
        .env("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json")
        .env(
            "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
            format!("http://127.0.0.1:{port}/v1/logs"),
        )
        .env(
            "OTEL_EXPORTER_OTLP_HEADERS",
            format!("Authorization=Bearer {token}"),
        )
        .env("OTEL_LOGS_EXPORT_INTERVAL", "1000");
    for name in ["NO_PROXY", "no_proxy"] {
        claude.env(name, without_proxy(env::var_os(name)));
    }
}

/// `no_proxy`, the value of a `NO_PROXY` variable, with `127.0.0.1` added, so a proxy never gets the events.
fn without_proxy(no_proxy: Option<OsString>) -> OsString {
    let mut hosts =
        no_proxy
            .filter(|hosts| !hosts.is_empty())
            .map_or_else(OsString::new, |mut hosts| {
                hosts.push(",");
                hosts
            });
    hosts.push("127.0.0.1");
    hosts
}

/// An event of a session that Cloneyard keeps, with only the attributes it counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The provider's id of the session it happened in.
    pub session_id: String,
    /// When it happened, in milliseconds since the Unix epoch.
    pub at: i64,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
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
    /// The input tokens that no cache served.
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// What Claude Code estimates the request costs, in millionths of a US dollar.
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
    /// How many times it was sent, the first time included.
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
    /// The kind of error of a failed call, such as `ShellError`.
    pub error_type: Option<String>,
    pub input_size_bytes: Option<i64>,
    pub result_size_bytes: Option<i64>,
}

/// Whether a tool call may run, and what decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDecision {
    pub tool_use_id: Option<String>,
    pub tool_name: String,
    /// `accept` or `reject`.
    pub decision: String,
    /// What decided, such as `config` or `user_reject`.
    pub source: Option<String>,
}

/// What an export holds for Cloneyard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Export {
    pub records: Vec<Record>,
    /// Why a record of an event Cloneyard keeps was dropped, one reason for each.
    pub dropped: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not an OTLP logs export in JSON: {0}")]
pub struct Invalid(String);

/// Reads the body of an OTLP/HTTP JSON logs export; a record without a time takes `now`, in milliseconds since the Unix epoch.
///
/// # Errors
///
/// When the body isn't the JSON of an export.
pub fn parse(json: &[u8], now: i64) -> Result<Export, Invalid> {
    let request: Request =
        serde_json::from_slice(json).map_err(|error| Invalid(error.to_string()))?;
    let mut export = Export::default();
    let records = request
        .resource_logs
        .into_iter()
        .flat_map(|resource| resource.scope_logs)
        .flat_map(|scope| scope.log_records);
    for record in records {
        let attributes = Attributes::of(&record.attributes);
        let body = record
            .body
            .as_ref()
            .and_then(|body| body.text.as_deref())
            .and_then(|body| body.strip_prefix("claude_code."));
        let Some(name) = attributes.text("event.name").or(body) else {
            continue;
        };
        let event = match name {
            "api_request" => Ok(Event::ApiRequest(api_request(&attributes))),
            "api_error" => Ok(Event::ApiError(api_error(&attributes))),
            "tool_result" => tool_result(&attributes).map(Event::ToolResult),
            "tool_decision" => tool_decision(&attributes).map(Event::ToolDecision),
            _ => continue,
        };
        let Some(session_id) = attributes.text("session.id").filter(|id| !id.is_empty()) else {
            export.dropped.push(format!("{name} without session.id"));
            continue;
        };
        match event {
            Ok(event) => export.records.push(Record {
                session_id: session_id.to_owned(),
                at: record.millis().unwrap_or(now),
                event,
            }),
            Err(missing) => export.dropped.push(format!("{name} without {missing}")),
        }
    }
    Ok(export)
}

fn api_request(attributes: &Attributes) -> ApiRequest {
    let count = |key| {
        attributes
            .integer(key)
            .filter(|count| *count >= 0)
            .unwrap_or(0)
    };
    ApiRequest {
        request_id: attributes.owned("request_id"),
        model: attributes.owned("model"),
        input_tokens: count("input_tokens"),
        output_tokens: count("output_tokens"),
        cache_read_tokens: count("cache_read_tokens"),
        cache_creation_tokens: count("cache_creation_tokens"),
        cost_usd_micros: count("cost_usd_micros"),
        duration_ms: attributes.integer("duration_ms"),
    }
}

fn api_error(attributes: &Attributes) -> ApiError {
    ApiError {
        request_id: attributes.owned("request_id"),
        model: attributes.owned("model"),
        error: attributes.owned("error"),
        status_code: attributes.integer("status_code"),
        attempt: attributes.integer("attempt"),
        duration_ms: attributes.integer("duration_ms"),
    }
}

fn tool_result(attributes: &Attributes) -> Result<ToolResult, &'static str> {
    Ok(ToolResult {
        tool_use_id: attributes.owned("tool_use_id"),
        tool_name: attributes.owned("tool_name").ok_or("tool_name")?,
        success: attributes.boolean("success").ok_or("success")?,
        duration_ms: attributes.integer("duration_ms"),
        error_type: attributes.owned("error_type"),
        input_size_bytes: attributes.integer("tool_input_size_bytes"),
        result_size_bytes: attributes.integer("tool_result_size_bytes"),
    })
}

fn tool_decision(attributes: &Attributes) -> Result<ToolDecision, &'static str> {
    Ok(ToolDecision {
        tool_use_id: attributes.owned("tool_use_id"),
        tool_name: attributes.owned("tool_name").ok_or("tool_name")?,
        decision: attributes.owned("decision").ok_or("decision")?,
        source: attributes.owned("source"),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    #[serde(default)]
    resource_logs: Vec<ResourceLogs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceLogs {
    #[serde(default)]
    scope_logs: Vec<ScopeLogs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeLogs {
    #[serde(default)]
    log_records: Vec<LogRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogRecord {
    time_unix_nano: Option<Integer>,
    observed_time_unix_nano: Option<Integer>,
    body: Option<AnyValue>,
    #[serde(default)]
    attributes: Vec<KeyValue>,
}

impl LogRecord {
    /// The time of the record, or else the time it was observed, in milliseconds; OTLP sends 0 for an unknown time.
    fn millis(&self) -> Option<i64> {
        [&self.time_unix_nano, &self.observed_time_unix_nano]
            .into_iter()
            .filter_map(|nanos| nanos.as_ref()?.value())
            .find(|&nanos| nanos > 0)
            .map(|nanos| nanos / 1_000_000)
    }
}

#[derive(Deserialize)]
struct KeyValue {
    key: String,
    #[serde(default)]
    value: AnyValue,
}

#[derive(Default, Deserialize)]
struct AnyValue {
    #[serde(rename = "stringValue")]
    text: Option<String>,
    #[serde(rename = "intValue")]
    integer: Option<Integer>,
    #[serde(rename = "boolValue")]
    boolean: Option<bool>,
}

/// An integer, which OTLP's JSON sends as a string, as it may not fit a JSON number.
#[derive(Deserialize)]
#[serde(untagged)]
enum Integer {
    Number(i64),
    Text(String),
}

impl Integer {
    fn value(&self) -> Option<i64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Text(text) => text.parse().ok(),
        }
    }
}

/// The attributes of a record by key; Claude Code sends some numbers and booleans as strings.
struct Attributes<'a>(HashMap<&'a str, &'a AnyValue>);

impl<'a> Attributes<'a> {
    fn of(list: &'a [KeyValue]) -> Self {
        Self(
            list.iter()
                .map(|attribute| (attribute.key.as_str(), &attribute.value))
                .collect(),
        )
    }

    fn text(&self, key: &str) -> Option<&'a str> {
        self.0.get(key)?.text.as_deref()
    }

    fn owned(&self, key: &str) -> Option<String> {
        self.text(key).map(str::to_owned)
    }

    fn integer(&self, key: &str) -> Option<i64> {
        let value = self.0.get(key)?;
        match (&value.integer, &value.text) {
            (Some(integer), _) => integer.value(),
            (None, Some(text)) => text.parse().ok(),
            (None, None) => None,
        }
    }

    fn boolean(&self, key: &str) -> Option<bool> {
        let value = self.0.get(key)?;
        match (value.boolean, value.text.as_deref()) {
            (Some(boolean), _) => Some(boolean),
            (None, Some(text)) => text.parse().ok(),
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use serde_json::{Value, json};

    use super::{
        ApiError, ApiRequest, Event, Export, Record, ToolDecision, ToolResult, parse, send_events,
        without_proxy,
    };

    const NOW: i64 = 1_795_674_800_000;

    /// An export of `records` in one resource and scope, as Claude Code sends it.
    fn export(records: &[Value]) -> Vec<u8> {
        json!({"resourceLogs": [{
            "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "claude-code"}}]},
            "scopeLogs": [{
                "scope": {"name": "com.anthropic.claude_code.events"},
                "logRecords": records,
            }],
        }]})
        .to_string()
        .into_bytes()
    }

    /// A record of event `name` at `millis`, with the attributes every event has and `attributes`.
    fn record(name: &str, millis: i64, attributes: &[(&str, Value)]) -> Value {
        let common = [
            ("session.id", json!({"stringValue": "2db7f4b9"})),
            ("user.email", json!({"stringValue": "ada@example.com"})),
            ("user.account_uuid", json!({"stringValue": "acc-uuid"})),
            ("organization.id", json!({"stringValue": "org-uuid"})),
            ("event.name", json!({"stringValue": name})),
            ("event.sequence", json!({"intValue": 13})),
        ];
        let attributes: Vec<Value> = common
            .iter()
            .chain(attributes)
            .map(|(key, value)| json!({"key": key, "value": value}))
            .collect();
        json!({
            "timeUnixNano": (millis * 1_000_000).to_string(),
            "body": {"stringValue": format!("claude_code.{name}")},
            "attributes": attributes,
        })
    }

    #[test]
    fn claude_sends_only_its_events_as_json_to_the_receiver_with_the_token() {
        let mut claude = crate::command(Path::new("/data/claude"));

        send_events(&mut claude, 7318, "f00d");

        let envs: HashMap<&OsStr, Option<&OsStr>> = claude.as_std().get_envs().collect();
        let value = |name: &str| envs.get(OsStr::new(name)).copied().flatten();
        let expected = [
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("OTEL_LOGS_EXPORTER", "otlp"),
            ("OTEL_METRICS_EXPORTER", "none"),
            ("OTEL_TRACES_EXPORTER", "none"),
            ("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/json"),
            (
                "OTEL_EXPORTER_OTLP_LOGS_ENDPOINT",
                "http://127.0.0.1:7318/v1/logs",
            ),
            ("OTEL_EXPORTER_OTLP_HEADERS", "Authorization=Bearer f00d"),
            ("OTEL_LOGS_EXPORT_INTERVAL", "1000"),
        ];
        for (name, set) in expected {
            assert_eq!(value(name), Some(OsStr::new(set)), "{name}");
        }
        for name in ["NO_PROXY", "no_proxy"] {
            let hosts = value(name).unwrap().to_str().unwrap();
            assert!(hosts.ends_with("127.0.0.1"), "{name}={hosts}");
        }
    }

    #[test]
    fn the_receiver_joins_the_hosts_that_bypass_a_proxy() {
        let joined = [
            None,
            Some(OsString::new()),
            Some(OsString::from("localhost,.internal")),
        ]
        .map(without_proxy);

        assert_eq!(
            joined,
            ["127.0.0.1", "127.0.0.1", "localhost,.internal,127.0.0.1"]
        );
    }

    #[test]
    fn an_api_request_keeps_its_tokens_cost_and_request_and_nothing_about_the_account() {
        let body = export(&[record(
            "api_request",
            NOW,
            &[
                ("model", json!({"stringValue": "claude-haiku-5-5"})),
                ("input_tokens", json!({"intValue": 2})),
                ("output_tokens", json!({"intValue": "10"})),
                ("cache_read_tokens", json!({"intValue": 4848})),
                ("cache_creation_tokens", json!({"intValue": 29335})),
                ("cache_creation_5m_tokens", json!({"intValue": -1})),
                ("cost_usd", json!({"doubleValue": 0.006_704})),
                ("cost_usd_micros", json!({"intValue": 6704})),
                ("duration_ms", json!({"intValue": 1014})),
                ("request_id", json!({"stringValue": "req_011"})),
            ],
        )]);

        let parsed = parse(&body, 0).unwrap();

        assert_eq!(
            parsed,
            Export {
                records: vec![Record {
                    session_id: "2db7f4b9".to_owned(),
                    at: NOW,
                    event: Event::ApiRequest(ApiRequest {
                        request_id: Some("req_011".to_owned()),
                        model: Some("claude-haiku-5-5".to_owned()),
                        input_tokens: 2,
                        output_tokens: 10,
                        cache_read_tokens: 4848,
                        cache_creation_tokens: 29335,
                        cost_usd_micros: 6704,
                        duration_ms: Some(1014),
                    }),
                }],
                dropped: vec![],
            }
        );
        let shown = format!("{parsed:?}");
        for identity in ["ada@example.com", "acc-uuid", "org-uuid"] {
            assert!(!shown.contains(identity), "{identity}");
        }
    }

    #[test]
    fn a_tool_result_reads_the_numbers_and_the_boolean_that_claude_code_sends_as_text() {
        let body = export(&[record(
            "tool_result",
            NOW,
            &[
                ("tool_name", json!({"stringValue": "Bash"})),
                ("tool_use_id", json!({"stringValue": "toolu_01"})),
                ("success", json!({"stringValue": "false"})),
                ("duration_ms", json!({"stringValue": "603"})),
                ("error_type", json!({"stringValue": "ShellError"})),
                ("tool_input_size_bytes", json!({"stringValue": "74"})),
                ("tool_result_size_bytes", json!({"stringValue": "11"})),
            ],
        )]);

        let parsed = parse(&body, 0).unwrap();

        assert_eq!(
            parsed.records[0].event,
            Event::ToolResult(ToolResult {
                tool_use_id: Some("toolu_01".to_owned()),
                tool_name: "Bash".to_owned(),
                success: false,
                duration_ms: Some(603),
                error_type: Some("ShellError".to_owned()),
                input_size_bytes: Some(74),
                result_size_bytes: Some(11),
            })
        );
    }

    #[test]
    fn a_tool_decision_and_an_api_error_keep_what_decided_and_what_failed() {
        let body = export(&[
            record(
                "tool_decision",
                NOW,
                &[
                    ("tool_name", json!({"stringValue": "Bash"})),
                    ("tool_use_id", json!({"stringValue": "toolu_01"})),
                    ("decision", json!({"stringValue": "reject"})),
                    ("source", json!({"stringValue": "config"})),
                ],
            ),
            record(
                "api_error",
                NOW,
                &[
                    ("error", json!({"stringValue": "Overloaded"})),
                    ("status_code", json!({"intValue": 529})),
                    ("attempt", json!({"intValue": 3})),
                ],
            ),
        ]);

        let parsed = parse(&body, 0).unwrap();

        let events: Vec<Event> = parsed
            .records
            .into_iter()
            .map(|record| record.event)
            .collect();
        assert_eq!(
            events,
            [
                Event::ToolDecision(ToolDecision {
                    tool_use_id: Some("toolu_01".to_owned()),
                    tool_name: "Bash".to_owned(),
                    decision: "reject".to_owned(),
                    source: Some("config".to_owned()),
                }),
                Event::ApiError(ApiError {
                    request_id: None,
                    model: None,
                    error: Some("Overloaded".to_owned()),
                    status_code: Some(529),
                    attempt: Some(3),
                    duration_ms: None,
                }),
            ]
        );
    }

    #[test]
    fn the_event_comes_from_the_body_without_event_name_and_other_events_are_skipped() {
        let mut from_body = record("api_request", NOW, &[]);
        from_body["attributes"]
            .as_array_mut()
            .unwrap()
            .retain(|attribute| attribute["key"] != "event.name");
        let body = export(&[
            record("user_prompt", NOW, &[]),
            from_body,
            json!({"body": {"stringValue": "something else"}}),
        ]);

        let parsed = parse(&body, 0).unwrap();

        assert_eq!(parsed.records.len(), 1);
        assert!(matches!(parsed.records[0].event, Event::ApiRequest(_)));
        assert_eq!(parsed.dropped, Vec::<String>::new());
    }

    #[test]
    fn a_record_without_a_session_or_a_tool_name_is_dropped_with_the_reason() {
        let mut lost = record("api_request", NOW, &[]);
        lost["attributes"]
            .as_array_mut()
            .unwrap()
            .retain(|attribute| attribute["key"] != "session.id");
        let body = export(&[
            lost,
            record(
                "tool_result",
                NOW,
                &[("success", json!({"boolValue": true}))],
            ),
        ]);

        let parsed = parse(&body, 0).unwrap();

        assert_eq!(parsed.records, []);
        assert_eq!(
            parsed.dropped,
            [
                "api_request without session.id",
                "tool_result without tool_name"
            ]
        );
    }

    #[test]
    fn a_negative_count_is_taken_as_none() {
        let body = export(&[record(
            "api_request",
            NOW,
            &[
                ("input_tokens", json!({"intValue": -5})),
                ("cost_usd_micros", json!({"intValue": "-1"})),
                ("output_tokens", json!({"intValue": 7})),
            ],
        )]);

        let parsed = parse(&body, 0).unwrap();

        let Event::ApiRequest(request) = &parsed.records[0].event else {
            panic!("{parsed:?}");
        };
        assert_eq!(
            (
                request.input_tokens,
                request.output_tokens,
                request.cost_usd_micros
            ),
            (0, 7, 0)
        );
    }

    #[test]
    fn a_record_without_a_time_takes_its_observed_time_and_else_now() {
        let mut observed = record("api_request", 0, &[]);
        observed["observedTimeUnixNano"] = json!((NOW - 1000) * 1_000_000);
        let mut untimed = record("api_request", 0, &[]);
        untimed.as_object_mut().unwrap().remove("timeUnixNano");
        let body = export(&[observed, untimed]);

        let parsed = parse(&body, NOW).unwrap();

        let times: Vec<i64> = parsed.records.iter().map(|record| record.at).collect();
        assert_eq!(times, [NOW - 1000, NOW]);
    }

    #[test]
    fn an_empty_export_holds_nothing_and_a_body_that_is_no_export_is_invalid() {
        let empty = parse(b"{}", NOW).unwrap();
        let errors: Vec<String> = [&b"not json"[..], br#"{"resourceLogs": {}}"#]
            .into_iter()
            .map(|body| parse(body, NOW).unwrap_err().to_string())
            .collect();

        assert_eq!(empty, Export::default());
        assert_eq!(
            errors,
            [
                "not an OTLP logs export in JSON: expected ident at line 1 column 2",
                "not an OTLP logs export in JSON: invalid type: map, expected a sequence at line 1 column 17",
            ]
        );
    }
}
