// Each test crate uses only some of these helpers.
#![allow(dead_code)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct Node {
    pub home: TempDir,
}

/// The `node.toml` of a test node, whose daemon takes a free telemetry port, as tests run side by side.
pub const NODE_TOML: &str = "[telemetry]\nport = 0\n";

impl Node {
    pub fn new() -> Self {
        let node = Self {
            // A clone writes in /tmp, so a repo there couldn't be hidden from it.
            home: TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap(),
        };
        let config = node.home.path().join("config").join("cloneyard");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("node.toml"), NODE_TOML).unwrap();
        node
    }

    pub fn cy(&self, subcommand: &str) -> Command {
        let home = self.home.path();
        let mut cy = Command::new(env!("CARGO_BIN_EXE_cy"));
        cy.arg(subcommand)
            .env_clear()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_RUNTIME_DIR", home)
            // The home is in this repo's target dir, and git would find this repo above it.
            .env("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"));
        if let Some(path) = env::var_os("PATH") {
            cy.env("PATH", path);
        }
        cy
    }

    pub fn socket(&self) -> PathBuf {
        self.home.path().join("cloneyard").join("daemon.sock")
    }

    pub fn start_daemon(&self) -> Daemon {
        self.spawn(self.cy("daemon"))
    }

    pub fn spawn(&self, mut daemon: Command) -> Daemon {
        let child = daemon
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon(child);
        wait_until("the daemon to listen", || {
            UnixStream::connect(self.socket()).is_ok()
        });
        daemon
    }
}

/// Kills the daemon when a test fails before it stops.
pub struct Daemon(pub Child);

impl Daemon {
    pub fn terminate(mut self) -> ExitStatus {
        let pid = self.0.id().to_string();
        let kill = Command::new("kill").args(["-TERM", &pid]).status().unwrap();
        assert!(kill.success());
        let mut status = None;
        wait_until("the daemon to exit", || {
            status = self.0.try_wait().unwrap();
            status.is_some()
        });
        status.unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn stderr_of_failure(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

pub fn stdout_of_success(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A new git repo at `path`, given as its canonical path.
pub fn repo(path: &Path) -> PathBuf {
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
    fs::canonicalize(path).unwrap()
}

pub fn project_add(node: &Node, path: &Path) -> Output {
    node.cy("project").arg("add").arg(path).output().unwrap()
}

pub const LOG_IN: &str = r#"echo '{"accessToken": "test"}' > "$CLAUDE_CONFIG_DIR/.credentials.json"
echo '{"numStartups": 0}' > "$CLAUDE_CONFIG_DIR/.claude.json""#;

pub const STATUS: &str = r#"if [ -f "$CLAUDE_CONFIG_DIR/.credentials.json" ]; then
    echo '{"loggedIn": true}'
else
    echo '{"loggedIn": false}'
    exit 1
fi"#;

/// Puts a `claude` in `bin/` whose `auth login` runs `login` and whose `auth status` reports a login when `.credentials.json` is in `$CLAUDE_CONFIG_DIR`.
pub fn fake_claude(node: &Node, login: &str) {
    fake_claude_with_status(node, login, STATUS);
}

pub fn fake_claude_with_status(node: &Node, login: &str, status: &str) {
    let bin = node.home.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let claude = bin.join("claude");
    fs::write(
        &claude,
        format!(
            r#"#!/bin/sh
case "$1 $2" in
"auth login")
{login}
;;
"auth status")
{status}
;;
*)
exit 2
;;
esac
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `bin/` in the node's home, where the fake `claude` is, before the inherited `PATH`.
pub fn bin_path(node: &Node) -> OsString {
    let mut path = OsString::from(node.home.path().join("bin"));
    if let Some(inherited) = env::var_os("PATH") {
        path.push(":");
        path.push(inherited);
    }
    path
}

/// `cy account add <name>`, which finds the fake `claude` first on its `PATH`.
pub fn account_add(node: &Node, name: &str) -> Command {
    let mut cy = node.cy("account");
    cy.args(["add", name]).env("PATH", bin_path(node));
    cy
}

pub fn accounts_dir(node: &Node) -> PathBuf {
    node.home.path().join("data/cloneyard/accounts")
}

/// The directory of account `name`, as the daemon records it.
pub fn claude_dir(node: &Node, name: &str) -> PathBuf {
    fs::canonicalize(accounts_dir(node))
        .unwrap()
        .join(name)
        .join("claude")
}
