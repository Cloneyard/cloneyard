use std::env;
use std::path::PathBuf;

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
