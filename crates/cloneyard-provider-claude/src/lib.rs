//! Claude Code as a provider: how Cloneyard starts `claude` for an account and reads what it prints.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use serde_json::json;
use tokio::process::Command;

pub mod headless;
pub mod hook;
pub mod telemetry;

/// `claude` with `dir` as its config directory, without the variables that would log it in some other way, tie it to a surrounding session or send its telemetry elsewhere.
#[must_use]
pub fn command(dir: &Path) -> Command {
    let mut claude = Command::new("claude");
    for (name, _) in std::env::vars_os() {
        if foreign(&name) {
            claude.env_remove(&name);
        }
    }
    claude.env(OsStr::new("CLAUDE_CONFIG_DIR"), dir);
    claude
}

/// Whether `claude` must not inherit the variable `name`.
fn foreign(name: &OsStr) -> bool {
    let bytes = name.as_bytes();
    bytes.starts_with(b"ANTHROPIC_") || bytes.starts_with(b"CLAUDE") || bytes.starts_with(b"OTEL_")
}

/// The `--settings` of a clone: hooks that run `program hook <event>`, which no settings file in the checkout can turn off, and, `inside_sandbox`, Claude Code's own sandbox off, as its `bwrap` can't start inside Cloneyard's.
#[must_use]
pub fn settings(program: &str, inside_sandbox: bool) -> String {
    let mut settings = json!({"hooks": hook::settings(program), "disableAllHooks": false});
    if inside_sandbox {
        settings["sandbox"] = json!({"enabled": false});
    }
    settings.to_string()
}

/// Turns off the auto-updater, which would replace the `claude` that runs outside Cloneyard's sandbox.
pub fn inside_sandbox(claude: &mut Command) {
    claude.env("DISABLE_AUTOUPDATER", "1");
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;

    use serde_json::{Value, json};

    use super::{command, foreign, inside_sandbox, settings};

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

    #[test]
    fn claude_inherits_no_login_session_or_telemetry_variable() {
        let dropped = [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CONFIG_DIR",
            "CLAUDECODE",
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            "OTEL_LOGS_EXPORTER",
            "PATH",
            "NO_PROXY",
        ]
        .map(|name| foreign(OsStr::new(name)));

        assert_eq!(dropped, [true, true, true, true, true, false, false]);
    }

    #[test]
    fn the_settings_keep_the_hooks_on_and_turn_claude_codes_sandbox_off_only_inside_ours() {
        let [outside, inside] = [false, true]
            .map(|inside| serde_json::from_str::<Value>(&settings("/bin/cy", inside)).unwrap());

        assert_eq!(outside["disableAllHooks"], json!(false));
        assert_eq!(outside.get("sandbox"), None);
        assert_eq!(inside["sandbox"], json!({"enabled": false}));
        assert_eq!(inside["hooks"], outside["hooks"]);
        assert_eq!(
            inside["hooks"]["Stop"][0]["hooks"][0]["command"],
            json!("'/bin/cy' hook Stop")
        );
    }

    #[test]
    fn inside_the_sandbox_claude_runs_without_its_auto_updater() {
        let mut claude = command(Path::new("/data/claude"));

        inside_sandbox(&mut claude);

        let updater = claude
            .as_std()
            .get_envs()
            .find(|(name, _)| *name == "DISABLE_AUTOUPDATER")
            .and_then(|(_, value)| value);
        assert_eq!(updater, Some(OsStr::new("1")));
    }
}
