use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::bail;
use cloneyard_api::clone::{
    Clone, Events, EventsParams, Handle, HandleParams, HookEvent, List, Run, RunParams, Show,
    State, Stop,
};

use crate::connection;

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
    let width = |column: fn(&Clone) -> String| {
        clones
            .iter()
            .map(|clone| column(clone).chars().count())
            .max()
            .unwrap_or_default()
    };
    let handle = width(|clone| clone.handle.to_string());
    let project = width(|clone| clone.project.clone());
    let account = width(|clone| clone.account.clone());
    let mode = width(|clone| clone.mode.to_string());
    let state = width(|clone| clone.state.to_string());
    for clone in &clones {
        let state = if clone.unsandboxed {
            format!("{:state$}  unsandboxed", clone.state.to_string())
        } else {
            clone.state.to_string()
        };
        println!(
            "{:handle$}  {:project$}  {:account$}  {:mode$}  {state}",
            clone.handle, clone.project, clone.account, clone.mode
        );
    }
    Ok(())
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
    lines.extend(
        clone
            .sessions
            .into_iter()
            .map(|session| ("session", session.provider_id)),
    );
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
    use cloneyard_api::clone::HookEvent;
    use ulid::Ulid;

    use super::event_lines;

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
}
