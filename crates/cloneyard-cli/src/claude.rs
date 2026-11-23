use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use anyhow::Context as _;
use cloneyard_provider_claude::command;
use serde_json::{Map, Value};
use tokio::signal::unix::{SignalKind, signal};

/// Runs `claude auth login` on this terminal with `dir` as its config directory, and tells whether `dir` holds a login afterwards.
pub(crate) fn log_in(dir: &Path) -> anyhow::Result<bool> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the tokio runtime")?;
    runtime.block_on(async {
        // Ctrl+C reaches claude as well; with a handler in place, cy outlives it and cleans up.
        let _interrupt = signal(SignalKind::interrupt()).context("cannot handle SIGINT")?;
        command(dir)
            .args(["auth", "login"])
            .status()
            .await
            .context("cannot run claude")?;
        let status = command(dir)
            .args(["auth", "status", "--json"])
            .output()
            .await
            .context("cannot run claude")?;
        let status: Value = serde_json::from_slice(&status.stdout).with_context(|| {
            format!(
                "invalid output of claude auth status: {}",
                String::from_utf8_lossy(&status.stderr).trim_end()
            )
        })?;
        Ok(status.get("loggedIn").and_then(Value::as_bool) == Some(true))
    })
}

/// Marks Claude Code's first-run screens, the theme picker and the welcome screen, as seen in `dir`.
pub(crate) fn skip_onboarding(dir: &Path) -> anyhow::Result<()> {
    let path = dir.join(".claude.json");
    let mut config: Map<String, Value> = match fs::read(&path) {
        Ok(json) => {
            serde_json::from_slice(&json).with_context(|| format!("invalid {}", path.display()))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    config.insert("hasCompletedOnboarding".to_owned(), Value::Bool(true));
    let json = serde_json::to_vec_pretty(&config)?;
    let temporary = dir.join(".claude.json.cloneyard");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(&json))
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    fs::rename(&temporary, &path).with_context(|| format!("cannot write {}", path.display()))
}
