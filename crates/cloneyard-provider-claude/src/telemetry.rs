//! Claude Code's OpenTelemetry: the variables that send a session's events to Cloneyard's receiver.

use std::env;
use std::ffi::OsString;

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use super::{send_events, without_proxy};

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
}
