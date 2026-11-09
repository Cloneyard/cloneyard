use std::env;
use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// `$XDG_CONFIG_HOME/cloneyard/node.toml`, or `~/.config/cloneyard/node.toml`.
///
/// # Errors
///
/// When neither `XDG_CONFIG_HOME` nor `HOME` is set.
pub fn config_file() -> anyhow::Result<PathBuf> {
    let config_home = xdg_dir("XDG_CONFIG_HOME")
        .or_else(|| env::home_dir().map(|home| home.join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(config_home.join("cloneyard").join("node.toml"))
}

// The XDG spec says to ignore a relative path in these variables.
fn xdg_dir(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// `$XDG_RUNTIME_DIR/cloneyard`, which holds the socket and the lock.
///
/// # Errors
///
/// When `XDG_RUNTIME_DIR` is unset or relative.
pub fn runtime_dir() -> anyhow::Result<PathBuf> {
    let runtime = xdg_dir("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not an absolute path, and the daemon socket lives there")?;
    Ok(runtime.join("cloneyard"))
}

#[must_use]
pub fn socket(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("daemon.sock")
}

pub(crate) fn lock(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("daemon.lock")
}
