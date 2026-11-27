//! The receiver of the clones' telemetry: OTLP/HTTP JSON on `127.0.0.1`, where a token of its own tells each clone.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use cloneyard_provider_claude::telemetry::{self, Event, Record};
use cloneyard_store::{
    ApiError, ApiRequest, NewTelemetry, TelemetryEvent, ToolDecision, ToolResult,
};
use serde_json::json;
use ulid::Ulid;
use uuid::Uuid;

use crate::node::Node;

/// The largest export the receiver reads, so no process can make the daemon hold a huge one.
const LIMIT: usize = 16 * 1024 * 1024;

/// The tokens of the running clones, by token.
#[derive(Default)]
pub(crate) struct Tokens(Mutex<HashMap<String, Ulid>>);

impl Tokens {
    /// A new token that tells the receiver clone `clone`.
    pub(crate) fn issue(&self, clone: Ulid) -> String {
        let token = Uuid::new_v4().simple().to_string();
        self.lock().insert(token.clone(), clone);
        token
    }

    /// Refuses the tokens of clone `clone` from now on.
    pub(crate) fn revoke(&self, clone: Ulid) {
        self.lock().retain(|_, owner| *owner != clone);
    }

    fn clone_of(&self, token: &str) -> Option<Ulid> {
        self.lock().get(token).copied()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Ulid>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The receiver's one route, `POST /v1/logs`.
pub(crate) fn router(node: Arc<Node>) -> Router {
    Router::new().route("/v1/logs", post(logs)).with_state(node)
}

async fn logs(State(node): State<Arc<Node>>, headers: HeaderMap, body: Body) -> Response {
    let Some(clone) = bearer(&headers).and_then(|token| node.tokens.clone_of(token)) else {
        return refuse(StatusCode::UNAUTHORIZED, "no token of a running clone");
    };
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    if !json {
        return refuse(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "only OTLP in JSON, as application/json",
        );
    }
    let Ok(body) = to_bytes(body, LIMIT).await else {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "an export of more than 16 MiB, or one cut short",
        );
    };
    let export = match telemetry::parse(&body, now()) {
        Ok(export) => export,
        Err(invalid) => return refuse(StatusCode::BAD_REQUEST, &invalid.to_string()),
    };
    let records = export.records.into_iter().map(record).collect();
    let refused = match node.store.add_telemetry(clone, records).await {
        Ok(refused) => refused,
        Err(error) => {
            tracing::error!(%clone, error = format!("{error:#}"), "cannot record telemetry");
            // Claude Code sends an export again only after 429, 502, 503 or 504.
            return refuse(StatusCode::SERVICE_UNAVAILABLE, "cannot record the export");
        }
    };
    let rejected: Vec<String> = export
        .dropped
        .into_iter()
        .chain(refused.iter().map(ToString::to_string))
        .collect();
    for reason in &rejected {
        tracing::warn!(%clone, reason, "dropped a telemetry record");
    }
    let answer = if rejected.is_empty() {
        json!({})
    } else {
        json!({"partialSuccess": {
            "rejectedLogRecords": rejected.len().to_string(),
            "errorMessage": rejected.join("; "),
        }})
    };
    (
        [(header::CONTENT_TYPE, "application/json")],
        answer.to_string(),
    )
        .into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, format!("{message}\n")).into_response()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or_default()
}

fn record(record: Record) -> NewTelemetry {
    let event = match record.event {
        Event::ApiRequest(request) => TelemetryEvent::ApiRequest(ApiRequest {
            request_id: request.request_id,
            model: request.model,
            input_tokens: request.input_tokens,
            output_tokens: request.output_tokens,
            cache_read_tokens: request.cache_read_tokens,
            cache_creation_tokens: request.cache_creation_tokens,
            cost_usd_micros: request.cost_usd_micros,
            duration_ms: request.duration_ms,
        }),
        Event::ApiError(error) => TelemetryEvent::ApiError(ApiError {
            request_id: error.request_id,
            model: error.model,
            error: error.error,
            status_code: error.status_code,
            attempt: error.attempt,
            duration_ms: error.duration_ms,
        }),
        Event::ToolResult(result) => TelemetryEvent::ToolResult(ToolResult {
            tool_use_id: result.tool_use_id,
            tool_name: result.tool_name,
            success: result.success,
            duration_ms: result.duration_ms,
            error_type: result.error_type,
            input_size_bytes: result.input_size_bytes,
            result_size_bytes: result.result_size_bytes,
        }),
        Event::ToolDecision(decision) => TelemetryEvent::ToolDecision(ToolDecision {
            tool_use_id: decision.tool_use_id,
            tool_name: decision.tool_name,
            decision: decision.decision,
            source: decision.source,
        }),
    };
    NewTelemetry {
        session: record.session_id,
        at: record.at,
        event,
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::sync::Arc;

    use cloneyard_api::clone::{Clone, Usage};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};

    use super::{LIMIT, router};
    use crate::node::Node;
    use crate::router::tests::open_node;
    use crate::spool::tests::add_clone;

    /// The receiver of `node` on a free port, which it gives.
    async fn serve(node: &Arc<Node>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = router(Arc::clone(node));
        tokio::spawn(async move { axum::serve(listener, app).await });
        port
    }

    /// The status and body of a `POST /v1/logs` with `headers` and `body`.
    async fn post(port: u16, headers: &[(&str, &str)], body: &[u8]) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut request = format!(
            "POST /v1/logs HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        for (name, value) in headers {
            write!(request, "{name}: {value}\r\n").unwrap();
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        // The receiver may answer a body it refuses before reading it all.
        let _ = stream.write_all(body).await;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let status = response[9..12].parse().unwrap();
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        (status, body.to_owned())
    }

    fn export(session: &str, request_id: &str) -> Vec<u8> {
        let attribute = |key: &str, value: Value| json!({"key": key, "value": value});
        json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{
            "timeUnixNano": "1795674808269000000",
            "body": {"stringValue": "claude_code.api_request"},
            "attributes": [
                attribute("event.name", json!({"stringValue": "api_request"})),
                attribute("session.id", json!({"stringValue": session})),
                attribute("user.email", json!({"stringValue": "ada@example.com"})),
                attribute("request_id", json!({"stringValue": request_id})),
                attribute("input_tokens", json!({"intValue": 2})),
                attribute("output_tokens", json!({"intValue": 10})),
                attribute("cache_read_tokens", json!({"intValue": 4848})),
                attribute("cache_creation_tokens", json!({"intValue": 29335})),
                attribute("cost_usd_micros", json!({"intValue": 6704})),
            ],
        }]}]}]})
        .to_string()
        .into_bytes()
    }

    async fn usage(node: &Node, clone: &Clone) -> Usage {
        node.store
            .clone_by_handle(node.id, clone.handle)
            .await
            .unwrap()
            .unwrap()
            .usage
    }

    const JSON: (&str, &str) = ("Content-Type", "application/json");

    #[tokio::test]
    async fn an_export_with_a_running_clones_token_adds_its_records_to_that_clone() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, _) = add_clone(&node, dir.path(), "s1").await;
        let token = node.tokens.issue(clone.id);
        let port = serve(&node).await;

        let answer = post(
            port,
            &[JSON, ("Authorization", &format!("Bearer {token}"))],
            &export("s1", "req_1"),
        )
        .await;

        assert_eq!(answer, (200, "{}".to_owned()));
        assert_eq!(
            usage(&node, &clone).await,
            Usage {
                input_tokens: 2,
                output_tokens: 10,
                cache_read_tokens: 4848,
                cache_creation_tokens: 29335,
                cost_usd_micros: 6704,
            }
        );
    }

    #[tokio::test]
    async fn an_export_without_the_token_of_a_running_clone_is_refused_and_adds_nothing() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, _) = add_clone(&node, dir.path(), "s1").await;
        let ended = node.tokens.issue(clone.id);
        node.tokens.revoke(clone.id);
        let port = serve(&node).await;

        let mut answers = Vec::new();
        for authorization in [
            None,
            Some("Bearer not-a-token".to_owned()),
            Some(format!("Bearer {ended}")),
            Some(ended.clone()),
        ] {
            let mut headers = vec![JSON];
            if let Some(authorization) = &authorization {
                headers.push(("Authorization", authorization));
            }
            answers.push(post(port, &headers, &export("s1", "req_1")).await);
        }

        let refused = (401, "no token of a running clone\n".to_owned());
        assert_eq!(
            answers,
            [refused.clone(), refused.clone(), refused.clone(), refused]
        );
        assert_eq!(usage(&node, &clone).await, Usage::default());
    }

    #[tokio::test]
    async fn an_export_that_is_not_json_too_large_or_not_otlp_is_refused() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, _) = add_clone(&node, dir.path(), "s1").await;
        let token = node.tokens.issue(clone.id);
        let bearer = format!("Bearer {token}");
        let port = serve(&node).await;

        let protobuf = post(
            port,
            &[
                ("Content-Type", "application/x-protobuf"),
                ("Authorization", &bearer),
            ],
            b"\x0a\x00",
        )
        .await;
        let large = post(
            port,
            &[JSON, ("Authorization", &bearer)],
            &vec![b' '; LIMIT + 1],
        )
        .await;
        let broken = post(port, &[JSON, ("Authorization", &bearer)], b"not json").await;

        assert_eq!(
            [protobuf, large, broken],
            [
                (415, "only OTLP in JSON, as application/json\n".to_owned()),
                (
                    413,
                    "an export of more than 16 MiB, or one cut short\n".to_owned()
                ),
                (
                    400,
                    "not an OTLP logs export in JSON: expected ident at line 1 column 2\n"
                        .to_owned()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_record_of_another_clones_session_is_rejected_as_a_partial_success() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, _) = add_clone(&node, dir.path(), "s1").await;
        let (other, _) = add_clone(&node, dir.path(), "s2").await;
        let token = node.tokens.issue(clone.id);
        let port = serve(&node).await;
        let bearer = format!("Bearer {token}");

        let foreign = post(
            port,
            &[JSON, ("Authorization", &bearer)],
            &export("s2", "req_1"),
        )
        .await;

        let (status, body) = foreign;
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap(),
            json!({"partialSuccess": {
                "rejectedLogRecords": "1",
                "errorMessage": format!("session s2 belongs to clone {}", other.id),
            }})
        );
        assert_eq!(usage(&node, &other).await, Usage::default());
    }
}
