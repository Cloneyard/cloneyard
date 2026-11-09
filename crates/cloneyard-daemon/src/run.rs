use std::env;
use std::future::Future;
use std::io::{self, IsTerminal as _};

use anyhow::Context as _;
use tokio::signal::unix::{SignalKind, signal};
use tracing_subscriber::EnvFilter;

use crate::{Config, Daemon, Log, paths};

/// Runs the daemon in the foreground until SIGTERM or SIGINT.
///
/// # Errors
///
/// When the settings are invalid or the daemon can't start, for example because another one runs.
pub fn run() -> anyhow::Result<()> {
    let config = Config::load(&paths::config_file()?)?;
    init_logging(&config.log)?;
    let runtime_dir = paths::runtime_dir()?;
    let data_dir = paths::data_dir()?;
    tokio::runtime::Runtime::new()
        .context("cannot start the tokio runtime")?
        .block_on(async {
            let shutdown = shutdown_signal()?;
            let daemon = Daemon::bind(&runtime_dir, &data_dir).await?;
            daemon.serve(shutdown).await
        })
}

fn init_logging(log: &Log) -> anyhow::Result<()> {
    let filter = match env::var_os(EnvFilter::DEFAULT_ENV).filter(|value| !value.is_empty()) {
        Some(directives) => EnvFilter::try_new(directives.to_string_lossy())
            .with_context(|| format!("invalid {}", EnvFilter::DEFAULT_ENV))?,
        None => EnvFilter::try_new(&log.level)?,
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .init();
    Ok(())
}

fn shutdown_signal() -> io::Result<impl Future<Output = ()>> {
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => tracing::info!("SIGTERM received"),
            _ = interrupt.recv() => tracing::info!("SIGINT received"),
        }
    })
}
