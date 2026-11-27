use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::bail;
use cloneyard_api::clone::{
    Events, EventsParams, Handle, HandleParams, HookEvent, List, Run, RunParams, Show, State, Stop,
    Usage,
};

use crate::connection;
use crate::output::{table, tokens};

// The checkout of a large repo takes a while.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

// Longer than the daemon gives a clone between SIGTERM and SIGKILL.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// How many of a clone's last events `cy clone show` prints.
const LAST_EVENTS: u32 = 10;

/// Starts a headless clone and prints its handle; a clone that could not start fails with its reason.
pub(crate) fn run(project: String, account: String, prompt: String) -> anyhow::Result<()> {
    let params = RunParams {
        project,
        account,
        prompt,
    };
    let clone = connection::with_client_within(RUN_TIMEOUT, async |client| {
        Ok(client.request::<Run>(params).await?)
    })?;
    println!("{}", clone.handle);
    if clone.state == State::Error {
        bail!(
            "clone {} did not start: {}",
            clone.handle,
            clone.reason.as_deref().unwrap_or("no reason given")
        );
    }
    Ok(())
}

pub(crate) fn list() -> anyhow::Result<()> {
    let clones =
        connection::with_client(async |client| Ok(client.request::<List>(()).await?))?.clones;
    let rows: Vec<Vec<String>> = clones
        .iter()
        .map(|clone| {
            let mut row = vec![
                clone.handle.to_string(),
                clone.project.clone(),
                clone.account.clone(),
                clone.mode.to_string(),
                clone.state.to_string(),
            ];
            row.extend(usage_cells(&clone.usage));
            if clone.unsandboxed {
                row.push("unsandboxed".to_owned());
            }
            row
        })
        .collect();
    for line in table(&rows, 5..9) {
        println!("{line}");
    }
    Ok(())
}

/// The input, output and cache tokens of `usage`, and its cost.
fn usage_cells(usage: &Usage) -> [String; 4] {
    [
        format!("{} in", tokens(usage.input_tokens)),
        format!("{} out", tokens(usage.output_tokens)),
        format!(
            "{} cache",
            tokens(usage.cache_read_tokens + usage.cache_creation_tokens)
        ),
        cost(usage.cost_usd_micros),
    ]
}

/// A cost in millionths of a US dollar, to four places below a dollar and to the cent from one.
fn cost(micros: u64) -> String {
    let ten_thousandths = (micros + 50) / 100;
    if ten_thousandths < 10_000 {
        return format!("$0.{ten_thousandths:04}");
    }
    let cents = (micros + 5_000) / 10_000;
    format!("${}.{:02}", cents / 100, cents % 100)
}

pub(crate) fn show(handle: Handle) -> anyhow::Result<()> {
    let (clone, events) = connection::with_client(async |client| {
        let clone = client.request::<Show>(HandleParams { handle }).await?;
        let params = EventsParams {
            handle,
            limit: LAST_EVENTS,
        };
        let events = client.request::<Events>(params).await?.events;
        Ok((clone, events))
    })?;
    let mut lines = vec![
        ("handle", clone.handle.to_string()),
        ("id", clone.id.to_string()),
        ("project", clone.project),
        ("account", clone.account),
        ("mode", clone.mode.to_string()),
        ("state", clone.state.to_string()),
    ];
    lines.extend(clone.reason.map(|reason| ("reason", reason)));
    lines.push((
        "sandbox",
        if clone.unsandboxed { "off" } else { "on" }.to_owned(),
    ));
    lines.push(("branch", clone.branch));
    lines.push(("checkout", clone.checkout));
    for session in clone.sessions {
        lines.push(("session", session.provider_id));
        lines.push(("usage", usage_cells(&session.usage).join("  ")));
    }
    lines.extend(event_lines(&events, now()));
    for (key, value) in lines {
        println!("{key:8}  {value}");
    }
    Ok(())
}

/// A line for each event, with its hook, its tool and how long ago it ran as of `now`, in milliseconds since the Unix epoch.
fn event_lines(events: &[HookEvent], now: i64) -> Vec<(&'static str, String)> {
    let labels: Vec<String> = events
        .iter()
        .map(|event| match &event.tool {
            Some(tool) => format!("{} {tool}", event.hook),
            None => event.hook.clone(),
        })
        .collect();
    let width = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or_default();
    events
        .iter()
        .zip(labels)
        .map(|(event, label)| {
            (
                "event",
                format!("{label:width$}  {} ago", age(now - event.at)),
            )
        })
        .collect()
}

fn age(millis: i64) -> String {
    let seconds = millis.max(0) / 1000;
    match seconds {
        ..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..172_800 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

pub(crate) fn stop(handle: Handle) -> anyhow::Result<()> {
    let clone = connection::with_client_within(STOP_TIMEOUT, async |client| {
        Ok(client.request::<Stop>(HandleParams { handle }).await?)
    })?;
    println!("{} {}", clone.state, clone.handle);
    Ok(())
}

#[cfg(test)]
mod tests {
    use cloneyard_api::clone::{HookEvent, Usage};
    use ulid::Ulid;

    use super::{cost, event_lines, usage_cells};

    fn event(hook: &str, tool: Option<&str>, at: i64) -> HookEvent {
        HookEvent {
            id: Ulid::generate(),
            at,
            session: "2f6c".to_owned(),
            hook: hook.to_owned(),
            tool: tool.map(str::to_owned),
        }
    }

    #[test]
    fn an_event_line_has_the_hook_its_tool_and_its_age_aligned() {
        let now = 1_000_000_000;
        let events = [
            event("UserPromptSubmit", None, now - 3 * 86_400_000),
            event("PreToolUse", Some("Bash"), now - 5 * 3_600_000),
            event("PostToolUse", Some("Bash"), now - 125_000),
            event("Stop", None, now - 999),
            event("SessionEnd", None, now + 2000),
        ];

        let lines: Vec<String> = event_lines(&events, now)
            .into_iter()
            .map(|(key, value)| format!("{key}  {value}"))
            .collect();

        assert_eq!(
            lines,
            [
                "event  UserPromptSubmit  3d ago",
                "event  PreToolUse Bash   5h ago",
                "event  PostToolUse Bash  2m ago",
                "event  Stop              0s ago",
                "event  SessionEnd        0s ago",
            ]
        );
    }

    #[test]
    fn a_cost_has_four_places_below_a_dollar_and_cents_from_one() {
        let shown = [
            0, 49, 6_704, 452_349, 999_949, 999_950, 1_234_567, 12_300_000,
        ]
        .map(cost);

        assert_eq!(
            shown,
            [
                "$0.0000", "$0.0000", "$0.0067", "$0.4523", "$0.9999", "$1.00", "$1.23", "$12.30"
            ]
        );
    }

    #[test]
    fn the_usage_adds_the_cache_tokens_read_and_written() {
        let usage = Usage {
            input_tokens: 2,
            output_tokens: 10,
            cache_read_tokens: 4848,
            cache_creation_tokens: 29335,
            cost_usd_micros: 6704,
        };

        assert_eq!(
            usage_cells(&usage),
            ["2 in", "10 out", "34.2k cache", "$0.0067"]
        );
    }
}
