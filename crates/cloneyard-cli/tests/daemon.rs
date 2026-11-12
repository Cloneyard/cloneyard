use std::fs;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;

const SCHEMA: &str = include_str!("../../../schema/cloneyard.openrpc.json");

struct Node {
    home: TempDir,
}

impl Node {
    fn new() -> Self {
        Self {
            home: TempDir::new().unwrap(),
        }
    }

    fn cy(&self, subcommand: &str) -> Command {
        let mut cy = Command::new(env!("CARGO_BIN_EXE_cy"));
        cy.arg(subcommand)
            .env("XDG_RUNTIME_DIR", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env_remove("RUST_LOG");
        cy
    }

    fn socket(&self) -> PathBuf {
        self.home.path().join("cloneyard").join("daemon.sock")
    }

    fn start_daemon(&self) -> Daemon {
        self.spawn(self.cy("daemon"))
    }

    fn spawn(&self, mut daemon: Command) -> Daemon {
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
struct Daemon(Child);

impl Daemon {
    fn terminate(mut self) -> ExitStatus {
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

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn stderr_of_failure(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// The node id in `cloneyard <version>, node <id>, round trip <ms> ms`.
fn node_id_from_ping(node: &Node) -> String {
    let output = node.cy("ping").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let prefix = concat!("cloneyard ", env!("CARGO_PKG_VERSION"), ", node ");
    let parsed = stdout.strip_prefix(prefix).and_then(|rest| {
        let (node_id, round_trip) = rest.split_once(", round trip ")?;
        let milliseconds = round_trip.strip_suffix(" ms\n")?;
        milliseconds.parse::<f64>().ok()?;
        Some(node_id.to_owned())
    });
    parsed.unwrap_or_else(|| panic!("unexpected output: {stdout:?}"))
}

#[test]
fn ping_prints_the_daemon_version_node_id_and_round_trip() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let node_id = node_id_from_ping(&node);

    assert_eq!(node_id.len(), 26, "{node_id}");
    assert!(
        node_id.chars().all(|c| c.is_ascii_alphanumeric()),
        "{node_id}"
    );
}

#[test]
fn the_node_id_survives_a_restart() {
    let node = Node::new();
    let daemon = node.start_daemon();
    let before = node_id_from_ping(&node);
    assert!(daemon.terminate().success());

    let _daemon = node.start_daemon();

    assert_eq!(node_id_from_ping(&node), before);
}

#[test]
fn a_second_daemon_refuses_to_start() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let stderr = stderr_of_failure(&node.cy("daemon").output().unwrap());

    let expected = format!(
        "error: cloneyard daemon is already running ({})\n",
        node.socket().display()
    );
    assert_eq!(stderr, expected);
}

#[test]
fn the_daemon_exits_cleanly_on_sigterm() {
    let node = Node::new();

    let status = node.start_daemon().terminate();

    assert!(status.success(), "{status}");
    assert!(!node.socket().exists());
}

#[test]
fn ping_fails_when_no_daemon_runs() {
    let node = Node::new();

    let stderr = stderr_of_failure(&node.cy("ping").output().unwrap());

    assert!(
        stderr.starts_with("error: cloneyard daemon is not running"),
        "{stderr}"
    );
}

#[test]
fn rpc_schema_prints_the_committed_schema() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let output = node.cy("rpc").arg("schema").output().unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), SCHEMA);
}

#[test]
fn rpc_schema_fails_like_ping_when_no_daemon_runs() {
    let node = Node::new();

    let schema = stderr_of_failure(&node.cy("rpc").arg("schema").output().unwrap());

    assert_eq!(
        schema,
        stderr_of_failure(&node.cy("ping").output().unwrap())
    );
}

#[test]
fn the_daemon_refuses_an_invalid_node_toml() {
    let node = Node::new();
    let config_dir = node.home.path().join("config").join("cloneyard");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("node.toml"), "colour = \"red\"\n").unwrap();

    let stderr = stderr_of_failure(&node.cy("daemon").output().unwrap());

    assert!(stderr.contains("node.toml"), "{stderr}");
    assert!(stderr.contains("unknown field `colour`"), "{stderr}");
}

#[test]
fn the_daemon_refuses_an_invalid_rust_log_and_says_why_once() {
    let node = Node::new();

    let output = node
        .cy("daemon")
        .env("RUST_LOG", "cloneyard=nonsense")
        .output()
        .unwrap();

    assert_eq!(
        stderr_of_failure(&output),
        concat!(
            "error: invalid RUST_LOG: error parsing level filter: expected one of ",
            "\"off\", \"error\", \"warn\", \"info\", \"debug\", \"trace\", or a number 0-5\n",
        )
    );
}

#[test]
fn an_invalid_rust_log_field_filter_keeps_the_regex_error() {
    let node = Node::new();

    let output = node
        .cy("daemon")
        .env("RUST_LOG", "[{x=/(}]")
        .output()
        .unwrap();

    assert_eq!(
        stderr_of_failure(&output),
        concat!(
            "error: invalid RUST_LOG: invalid field filter: error building NFA: ",
            "error parsing regex: regex parse error:\n",
            "    /(\n",
            "     ^\n",
            "error: unclosed group\n",
        )
    );
}

#[test]
fn commands_stop_without_an_absolute_xdg_runtime_dir() {
    let node = Node::new();

    let unset = node
        .cy("ping")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    let relative = node
        .cy("ping")
        .env("XDG_RUNTIME_DIR", "run")
        .output()
        .unwrap();

    for output in [unset, relative] {
        let stderr = stderr_of_failure(&output);
        assert!(
            stderr.contains("XDG_RUNTIME_DIR is not an absolute path"),
            "{stderr}"
        );
    }
}

#[test]
fn reads_node_toml_from_home_when_xdg_config_home_is_relative() {
    let node = Node::new();
    let config_dir = node.home.path().join(".config").join("cloneyard");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join("node.toml"), "colour = \"red\"\n").unwrap();

    let output = node
        .cy("daemon")
        .env("HOME", node.home.path())
        .env("XDG_CONFIG_HOME", "config")
        .output()
        .unwrap();

    let stderr = stderr_of_failure(&output);
    assert!(
        stderr.contains(&config_dir.join("node.toml").display().to_string()),
        "{stderr}"
    );
}

#[test]
fn keeps_the_database_in_home_when_xdg_data_home_is_relative() {
    let node = Node::new();
    let mut daemon = node.cy("daemon");
    daemon
        .env("HOME", node.home.path())
        .env("XDG_DATA_HOME", "data");

    let _daemon = node.spawn(daemon);

    let database = node.home.path().join(".local/share/cloneyard/cloneyard.db");
    assert!(database.exists(), "{}", database.display());
}
