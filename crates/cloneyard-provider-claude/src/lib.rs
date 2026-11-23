//! Claude Code as a provider: how Cloneyard starts `claude` for an account and reads what it prints.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use tokio::process::Command;

pub mod headless;

/// `claude` with `dir` as its config directory, without the variables that would log it in some other way or tie it to a surrounding session.
#[must_use]
pub fn command(dir: &Path) -> Command {
    let mut claude = Command::new("claude");
    for (name, _) in std::env::vars_os() {
        let bytes = name.as_bytes();
        if bytes.starts_with(b"ANTHROPIC_") || bytes.starts_with(b"CLAUDE") {
            claude.env_remove(&name);
        }
    }
    claude.env(OsStr::new("CLAUDE_CONFIG_DIR"), dir);
    claude
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use super::command;

    #[test]
    fn the_command_has_the_account_directory_as_its_config_directory() {
        let claude = command(Path::new("/data/accounts/personal/claude"));

        let config_dir = claude
            .as_std()
            .get_envs()
            .find(|(name, _)| *name == "CLAUDE_CONFIG_DIR")
            .and_then(|(_, value)| value);
        assert_eq!(
            config_dir,
            Some(OsStr::new("/data/accounts/personal/claude"))
        );
    }
}
