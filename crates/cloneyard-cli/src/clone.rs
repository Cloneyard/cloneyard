use std::time::Duration;

use anyhow::bail;
use cloneyard_api::clone::{Clone, Handle, HandleParams, List, Run, RunParams, Show, State, Stop};

use crate::connection;

// The checkout of a large repo takes a while.
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

// Longer than the daemon gives a clone between SIGTERM and SIGKILL.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

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
    for clone in &clones {
        println!(
            "{:handle$}  {:project$}  {:account$}  {:mode$}  {}",
            clone.handle, clone.project, clone.account, clone.mode, clone.state
        );
    }
    Ok(())
}

pub(crate) fn show(handle: Handle) -> anyhow::Result<()> {
    let clone = connection::with_client(async |client| {
        Ok(client.request::<Show>(HandleParams { handle }).await?)
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
    lines.push(("branch", clone.branch));
    lines.push(("checkout", clone.checkout));
    lines.extend(
        clone
            .sessions
            .into_iter()
            .map(|session| ("session", session.provider_id)),
    );
    for (key, value) in lines {
        println!("{key:8}  {value}");
    }
    Ok(())
}

pub(crate) fn stop(handle: Handle) -> anyhow::Result<()> {
    let clone = connection::with_client_within(STOP_TIMEOUT, async |client| {
        Ok(client.request::<Stop>(HandleParams { handle }).await?)
    })?;
    println!("{} {}", clone.state, clone.handle);
    Ok(())
}
