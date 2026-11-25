use std::env;
use std::io::{self, Read as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use cloneyard_daemon::spool;

/// Writes the JSON on stdin into the clone's spool; it always succeeds and prints only to stderr, since Claude Code adds a hook's stdout to the session and waits for its exit.
pub(crate) fn run(event: &str) -> ExitCode {
    if let Err(error) = write(event) {
        eprintln!("cy hook {event}: {error:#}");
    }
    ExitCode::SUCCESS
}

fn write(event: &str) -> anyhow::Result<()> {
    let dir =
        env::var_os(spool::VARIABLE).with_context(|| format!("{} is not set", spool::VARIABLE))?;
    let mut payload = Vec::new();
    io::stdin()
        .read_to_end(&mut payload)
        .context("cannot read stdin")?;
    spool::write(Path::new(&dir), event, &payload)?;
    Ok(())
}
