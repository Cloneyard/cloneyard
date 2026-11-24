//! The sandbox a clone runs in: Landlock limits what it reads and writes, and seccomp filters keep it off Unix sockets other than datagram ones.
//!
//! The daemon starts a clone through `command`, as `cy sandbox <policy> -- <program>`, and that process calls `enter`, which restricts itself and then runs the program; the daemon stays outside.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

mod files;
mod policy;
mod sockets;

pub use policy::Policy;

/// Why a policy is invalid, or a clone can't enter its sandbox; each message holds its cause, so none has a source.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} is not an absolute path in UTF-8", .0.display())]
    NotAbsolute(PathBuf),
    #[error("cannot hide {} inside {}, which the clone can read", hidden.display(), granted.display())]
    HiddenInside { hidden: PathBuf, granted: PathBuf },
    #[error("invalid policy: {0}")]
    Policy(serde_json::Error),
    #[error("cannot open {}: {error}", path.display())]
    Open { path: PathBuf, error: io::Error },
    #[error("cannot apply Landlock: {0}")]
    Landlock(landlock::RulesetError),
    #[error("this kernel has no Landlock")]
    NoLandlock,
    #[error("cannot apply the seccomp filter: {0}")]
    Seccomp(seccompiler::Error),
    #[error("cannot run {program}: {error}")]
    Exec { program: String, error: io::Error },
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Policy(error)
    }
}

impl From<landlock::RulesetError> for Error {
    fn from(error: landlock::RulesetError) -> Self {
        Self::Landlock(error)
    }
}

impl From<seccompiler::Error> for Error {
    fn from(error: seccompiler::Error) -> Self {
        Self::Seccomp(error)
    }
}

impl From<seccompiler::BackendError> for Error {
    fn from(error: seccompiler::BackendError) -> Self {
        Self::Seccomp(error.into())
    }
}

/// Whether the kernel has Landlock, which the sandbox needs.
#[must_use]
pub fn supported() -> bool {
    files::supported()
}

/// `exe sandbox <policy> -- <program> <args>` for `inner`, with its environment and directory, and its program found on its `PATH` now, so a missing one fails here.
///
/// # Errors
///
/// When the program isn't on the `PATH`.
pub fn command(exe: &Path, policy: &Policy, inner: &Command) -> io::Result<Command> {
    let program = find(inner)?;
    let mut outer = Command::new(exe);
    outer
        .arg("sandbox")
        .arg(serde_json::to_string(policy).map_err(io::Error::other)?)
        .arg("--")
        .arg(program)
        .args(inner.get_args());
    for (name, value) in inner.get_envs() {
        match value {
            Some(value) => outer.env(name, value),
            None => outer.env_remove(name),
        };
    }
    if let Some(dir) = inner.get_current_dir() {
        outer.current_dir(dir);
    }
    Ok(outer)
}

fn find(inner: &Command) -> io::Result<PathBuf> {
    let program = Path::new(inner.get_program());
    if program.components().count() > 1 {
        return Ok(program.to_owned());
    }
    let path = match inner.get_envs().find(|(name, _)| *name == "PATH") {
        Some((_, value)) => value.map(OsStr::to_owned),
        None => env::var_os("PATH"),
    };
    env::split_paths(&path.unwrap_or_default())
        .map(|dir| dir.join(program))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .ok_or_else(|| io::Error::from_raw_os_error(libc::ENOENT))
}

/// Restricts this process to the policy in JSON, then replaces it with `program`, which inherits no descriptor but stdin, stdout and stderr; it returns only when one of the two fails.
#[must_use]
pub fn enter(policy: &str, program: &OsStr, args: &[OsString]) -> Error {
    if let Err(error) = restrict(policy) {
        return error;
    }
    let error = Command::new(program).args(args).exec();
    Error::Exec {
        program: program.to_string_lossy().into_owned(),
        error,
    }
}

fn restrict(policy: &str) -> Result<(), Error> {
    let policy: Policy = serde_json::from_str(policy)?;
    // The daemon can inherit descriptors from whatever started it, so none but stdio may reach the program.
    close_fds::set_fds_cloexec(3, &[]);
    files::restrict(&policy)?;
    sockets::filter()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

    use super::{Policy, command};

    #[test]
    fn the_command_runs_the_program_found_on_its_path_through_exe_with_its_env_and_dir() {
        let dir = TempDir::new().unwrap();
        let claude = dir.path().join("claude");
        fs::write(&claude, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        let policy = Policy::new(Vec::new(), Vec::new(), vec!["/tmp".into()]).unwrap();
        let mut inner = Command::new("claude");
        inner
            .args(["-p", "--verbose"])
            .env("PATH", dir.path())
            .env("CLAUDE_CONFIG_DIR", "/data/claude")
            .env_remove("ANTHROPIC_API_KEY")
            .current_dir("/code/app");

        let outer = command(Path::new("/proc/self/exe"), &policy, &inner).unwrap();

        let args: Vec<&OsStr> = outer.get_args().collect();
        let envs: Vec<_> = outer.get_envs().collect();
        assert_eq!(outer.get_program(), "/proc/self/exe");
        assert_eq!(
            args,
            [
                OsStr::new("sandbox"),
                OsStr::new(r#"{"hidden":[],"visible":[],"writable":["/tmp"]}"#),
                OsStr::new("--"),
                claude.as_os_str(),
                OsStr::new("-p"),
                OsStr::new("--verbose"),
            ]
        );
        assert!(envs.contains(&(
            OsStr::new("CLAUDE_CONFIG_DIR"),
            Some(OsStr::new("/data/claude"))
        )));
        assert!(envs.contains(&(OsStr::new("ANTHROPIC_API_KEY"), None)));
        assert_eq!(outer.get_current_dir(), Some(Path::new("/code/app")));
    }

    #[test]
    fn a_program_missing_from_the_path_fails_as_spawning_it_would() {
        let dir = TempDir::new().unwrap();
        let policy = Policy::new(Vec::new(), Vec::new(), Vec::new()).unwrap();
        let mut inner = Command::new("claude");
        inner.env("PATH", dir.path());

        let error = command(Path::new("/proc/self/exe"), &policy, &inner).unwrap_err();

        assert_eq!(error.to_string(), "No such file or directory (os error 2)");
    }
}
