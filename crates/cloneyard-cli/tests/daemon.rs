use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
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
        let home = self.home.path();
        let mut cy = Command::new(env!("CARGO_BIN_EXE_cy"));
        cy.arg(subcommand)
            .env_clear()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_RUNTIME_DIR", home);
        if let Some(path) = env::var_os("PATH") {
            cy.env("PATH", path);
        }
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

fn stdout_of_success(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// A new git repo at `path`, given as its canonical path.
fn repo(path: &Path) -> PathBuf {
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
    fs::canonicalize(path).unwrap()
}

fn project_add(node: &Node, path: &Path) -> Output {
    node.cy("project").arg("add").arg(path).output().unwrap()
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
    daemon.env("XDG_DATA_HOME", "data");

    let _daemon = node.spawn(daemon);

    let database = node.home.path().join(".local/share/cloneyard/cloneyard.db");
    assert!(database.exists(), "{}", database.display());
}

#[test]
fn project_add_adds_the_current_directory_and_prints_the_name_and_id_of_the_project() {
    let node = Node::new();
    let app = repo(&node.home.path().join("app"));
    let _daemon = node.start_daemon();

    let output = node
        .cy("project")
        .args(["add", "."])
        .current_dir(&app)
        .output()
        .unwrap();

    let stdout = stdout_of_success(&output);
    let id = stdout
        .strip_prefix("added project app (")
        .and_then(|rest| rest.strip_suffix(")\n"))
        .unwrap_or_else(|| panic!("unexpected output: {stdout:?}"));
    assert_eq!(id.len(), 26, "{id}");
}

#[test]
fn projects_lists_the_projects_with_their_paths_after_a_restart() {
    let node = Node::new();
    let web = repo(&node.home.path().join("web"));
    let api = repo(&node.home.path().join("code").join("api"));
    let daemon = node.start_daemon();
    stdout_of_success(&project_add(&node, &web));
    stdout_of_success(
        &node
            .cy("project")
            .arg("add")
            .arg(&api)
            .args(["--name", "backend"])
            .output()
            .unwrap(),
    );
    assert!(daemon.terminate().success());
    let _daemon = node.start_daemon();

    let stdout = stdout_of_success(&node.cy("projects").output().unwrap());

    assert_eq!(
        stdout,
        format!("backend  {}\nweb      {}\n", api.display(), web.display())
    );
}

#[test]
fn projects_prints_nothing_without_projects() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let stdout = stdout_of_success(&node.cy("projects").output().unwrap());

    assert_eq!(stdout, "");
}

#[test]
fn project_add_names_the_path_that_is_not_in_a_git_working_tree() {
    let node = Node::new();
    let notes = node.home.path().join("notes");
    fs::create_dir(&notes).unwrap();
    let _daemon = node.start_daemon();

    let stderr = stderr_of_failure(&project_add(&node, &notes));

    assert_eq!(
        stderr,
        format!("error: {} is not in a git working tree\n", notes.display())
    );
}

#[test]
fn project_add_names_the_project_that_has_the_repo_already() {
    let node = Node::new();
    let app = repo(&node.home.path().join("app"));
    let _daemon = node.start_daemon();
    stdout_of_success(&project_add(&node, &app));

    let output = node
        .cy("project")
        .arg("add")
        .arg(&app)
        .args(["--name", "other"])
        .output()
        .unwrap();

    assert_eq!(
        stderr_of_failure(&output),
        format!("error: {} is already in project app\n", app.display())
    );
}

#[test]
fn project_add_names_the_project_whose_name_is_taken() {
    let node = Node::new();
    let first = repo(&node.home.path().join("code").join("app"));
    let second = repo(&node.home.path().join("work").join("app"));
    let _daemon = node.start_daemon();
    stdout_of_success(&project_add(&node, &first));

    let stderr = stderr_of_failure(&project_add(&node, &second));

    assert_eq!(stderr, "error: project app already exists\n");
}

#[test]
fn project_add_fails_as_an_internal_error_when_the_daemon_finds_no_git() {
    let node = Node::new();
    let app = repo(&node.home.path().join("app"));
    let mut daemon = node.cy("daemon");
    daemon.env("PATH", node.home.path().join("empty"));
    let _daemon = node.spawn(daemon);

    let stderr = stderr_of_failure(&project_add(&node, &app));

    assert_eq!(
        stderr,
        format!(
            "error: Internal error (-32603): cannot run git in {}: No such file or directory (os error 2)\n",
            app.display()
        )
    );
}

#[test]
fn project_add_refuses_a_path_that_is_not_utf_8() {
    let node = Node::new();
    let path = node.home.path().join(OsStr::from_bytes(b"caf\xe9"));

    let stderr = stderr_of_failure(&project_add(&node, &path));

    assert_eq!(
        stderr,
        format!("error: {} is not valid UTF-8\n", path.display())
    );
}

const LOG_IN: &str = r#"echo '{"accessToken": "test"}' > "$CLAUDE_CONFIG_DIR/.credentials.json"
echo '{"numStartups": 0}' > "$CLAUDE_CONFIG_DIR/.claude.json""#;

const STATUS: &str = r#"if [ -f "$CLAUDE_CONFIG_DIR/.credentials.json" ]; then
    echo '{"loggedIn": true}'
else
    echo '{"loggedIn": false}'
    exit 1
fi"#;

/// Puts a `claude` in `bin/` whose `auth login` runs `login` and whose `auth status` reports a login when `.credentials.json` is in `$CLAUDE_CONFIG_DIR`.
fn fake_claude(node: &Node, login: &str) {
    fake_claude_with_status(node, login, STATUS);
}

fn fake_claude_with_status(node: &Node, login: &str, status: &str) {
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

/// `cy account add <name>`, which finds the fake `claude` first on its `PATH`.
fn account_add(node: &Node, name: &str) -> Command {
    let mut path = OsString::from(node.home.path().join("bin"));
    if let Some(inherited) = env::var_os("PATH") {
        path.push(":");
        path.push(inherited);
    }
    let mut cy = node.cy("account");
    cy.args(["add", name]).env("PATH", path);
    cy
}

fn accounts_dir(node: &Node) -> PathBuf {
    node.home.path().join("data/cloneyard/accounts")
}

/// The directory of account `name`, as the daemon records it.
fn claude_dir(node: &Node, name: &str) -> PathBuf {
    fs::canonicalize(accounts_dir(node))
        .unwrap()
        .join(name)
        .join("claude")
}

fn assert_no_accounts(node: &Node) {
    assert_eq!(
        stdout_of_success(&node.cy("accounts").output().unwrap()),
        ""
    );
    let left: Vec<PathBuf> = fs::read_dir(accounts_dir(node))
        .map_or_default(|entries| entries.map(|entry| entry.unwrap().path()).collect());
    assert_eq!(left, Vec::<PathBuf>::new());
}

/// The account id in `added account <name> (<id>)`, after the greeting when `greeted`.
fn id_from_account_add(stdout: &str, name: &str, greeted: bool) -> String {
    let rest = if greeted {
        stdout.strip_prefix("Hello there.\n")
    } else {
        Some(stdout)
    };
    let id = rest
        .and_then(|rest| rest.strip_prefix(&format!("added account {name} (")))
        .and_then(|rest| rest.strip_suffix(")\n"))
        .unwrap_or_else(|| panic!("unexpected output: {stdout:?}"));
    assert_eq!(id.len(), 26, "{id}");
    id.to_owned()
}

#[test]
fn account_add_greets_the_first_account_only_and_prints_the_name_and_id() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let _daemon = node.start_daemon();

    let first = stdout_of_success(&account_add(&node, "personal").output().unwrap());
    let second = stdout_of_success(&account_add(&node, "work").output().unwrap());

    id_from_account_add(&first, "personal", true);
    id_from_account_add(&second, "work", false);
}

#[test]
fn accounts_lists_the_accounts_with_their_providers_and_directories_after_a_restart() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let daemon = node.start_daemon();
    stdout_of_success(&account_add(&node, "work").output().unwrap());
    stdout_of_success(&account_add(&node, "me").output().unwrap());
    assert!(daemon.terminate().success());
    let _daemon = node.start_daemon();

    let stdout = stdout_of_success(&node.cy("accounts").output().unwrap());

    assert_eq!(
        stdout,
        format!(
            "me    claude-code  {}\nwork  claude-code  {}\n",
            claude_dir(&node, "me").display(),
            claude_dir(&node, "work").display()
        )
    );
}

#[test]
fn accounts_prints_nothing_without_accounts() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let stdout = stdout_of_success(&node.cy("accounts").output().unwrap());

    assert_eq!(stdout, "");
}

#[test]
fn account_add_marks_the_onboarding_done_and_keeps_what_the_login_wrote() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let _daemon = node.start_daemon();

    stdout_of_success(&account_add(&node, "personal").output().unwrap());

    let config = claude_dir(&node, "personal").join(".claude.json");
    let json: serde_json::Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"hasCompletedOnboarding": true, "numStartups": 0})
    );
    let mode = fs::metadata(&config).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
}

#[test]
fn account_add_refuses_a_taken_name_before_claude_runs() {
    let node = Node::new();
    fake_claude(&node, &format!("echo login >> \"$HOME/logins\"\n{LOG_IN}"));
    let _daemon = node.start_daemon();
    stdout_of_success(&account_add(&node, "personal").output().unwrap());

    let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

    assert_eq!(stderr, "error: account personal already exists\n");
    let logins = fs::read_to_string(node.home.path().join("logins")).unwrap();
    assert_eq!(logins, "login\n");
}

#[test]
fn account_add_refuses_a_name_that_is_not_one_path_component_before_anything_else() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);

    for name in ["", ".", "..", "me/work"] {
        let stderr = stderr_of_failure(&account_add(&node, name).output().unwrap());

        assert_eq!(
            stderr,
            format!(
                "error: invalid account name {name:?}: it names a directory, \
                 so it can't be empty, \".\", \"..\" or contain \"/\"\n"
            )
        );
    }
    assert!(!accounts_dir(&node).exists());
}

#[test]
fn account_add_adds_nothing_when_the_login_fails_or_logs_nothing_in() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    for login in ["exit 1", "exit 0"] {
        fake_claude(&node, login);

        let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

        assert_eq!(
            stderr, "error: claude auth login did not log in, so account personal was not added\n",
            "{login}"
        );
        assert_no_accounts(&node);
    }
}

#[test]
fn account_add_adds_nothing_when_ctrl_c_stops_the_login() {
    let node = Node::new();
    let started = node.home.path().join("login-started");
    fake_claude(&node, "touch \"$HOME/login-started\"\nexec sleep 30");
    let _daemon = node.start_daemon();
    let cy = account_add(&node, "personal")
        .process_group(0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_until("the login to start", || started.exists());

    let group = format!("-{}", cy.id());
    let kill = Command::new("kill")
        .args(["-INT", "--", &group])
        .status()
        .unwrap();
    let output = cy.wait_with_output().unwrap();

    assert!(kill.success());
    assert_eq!(
        stderr_of_failure(&output),
        "error: claude auth login did not log in, so account personal was not added\n"
    );
    assert_no_accounts(&node);
}

#[test]
fn account_add_adds_nothing_when_the_daemon_stops_during_the_login() {
    let node = Node::new();
    let daemon = node.start_daemon();
    let socket = node.socket();
    fake_claude(
        &node,
        &format!(
            "{LOG_IN}\nkill -TERM {}\nwhile [ -e '{}' ]; do sleep 0.01; done",
            daemon.0.id(),
            socket.display()
        ),
    );

    let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

    assert_eq!(
        stderr,
        format!(
            "error: cloneyard daemon is not running ({})\n",
            socket.display()
        )
    );
    assert!(!accounts_dir(&node).join("personal").exists());
}

#[test]
fn account_add_names_the_directory_it_cannot_remove_after_a_failed_login() {
    let node = Node::new();
    fake_claude(
        &node,
        "touch \"$CLAUDE_CONFIG_DIR/cache\"\nchmod 500 \"$CLAUDE_CONFIG_DIR\"",
    );
    let _daemon = node.start_daemon();

    let output = account_add(&node, "work").output().unwrap();

    let home = accounts_dir(&node).join("work");
    fs::set_permissions(home.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        stderr_of_failure(&output),
        format!(
            "error: claude auth login did not log in, so account work was not added; \
             also cannot remove {}: Permission denied (os error 13)\n",
            home.display()
        )
    );
}

#[test]
fn account_add_leaves_a_directory_it_did_not_create_alone() {
    let node = Node::new();
    let home = accounts_dir(&node).join("personal");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("notes"), "mine\n").unwrap();
    fake_claude(&node, &format!("touch \"$HOME/login-ran\"\n{LOG_IN}"));
    let _daemon = node.start_daemon();

    let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

    assert_eq!(
        stderr,
        format!(
            "error: cannot create {}: File exists (os error 17)\n",
            home.display()
        )
    );
    assert_eq!(fs::read_to_string(home.join("notes")).unwrap(), "mine\n");
    assert!(!node.home.path().join("login-ran").exists());
}

#[test]
fn account_add_runs_claude_without_the_variables_that_would_log_it_in_another_way() {
    let node = Node::new();
    fake_claude(&node, &format!("env > \"$HOME/claude-env\"\n{LOG_IN}"));
    let _daemon = node.start_daemon();

    let output = account_add(&node, "personal")
        .env("ANTHROPIC_API_KEY", "sk-ant-test")
        .env("ANTHROPIC_AUTH_TOKEN", "test")
        .env("CLAUDE_CODE_OAUTH_TOKEN", "test")
        .env("CLAUDE_CONFIG_DIR", node.home.path().join(".claude"))
        .env("CLAUDECODE", "1")
        .output()
        .unwrap();

    stdout_of_success(&output);
    let env = fs::read_to_string(node.home.path().join("claude-env")).unwrap();
    let names: Vec<&str> = env
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .filter(|name| name.starts_with("ANTHROPIC_") || name.starts_with("CLAUDE"))
        .collect();
    assert_eq!(names, ["CLAUDE_CONFIG_DIR"]);
    let config_dir = accounts_dir(&node).join("personal").join("claude");
    assert!(
        env.contains(&format!("CLAUDE_CONFIG_DIR={}\n", config_dir.display())),
        "{env}"
    );
}

#[test]
fn account_add_fails_without_claude_and_adds_nothing() {
    let node = Node::new();
    let _daemon = node.start_daemon();

    let output = node
        .cy("account")
        .args(["add", "personal"])
        .env("PATH", node.home.path().join("empty"))
        .output()
        .unwrap();

    assert_eq!(
        stderr_of_failure(&output),
        "error: cannot run claude: No such file or directory (os error 2)\n"
    );
    assert_no_accounts(&node);
}

#[test]
fn account_add_names_claude_auth_status_when_it_prints_no_json() {
    let node = Node::new();
    fake_claude_with_status(&node, LOG_IN, "echo 'no status' >&2\nexit 1");
    let _daemon = node.start_daemon();

    let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

    assert_eq!(
        stderr,
        "error: invalid output of claude auth status: no status: \
         EOF while parsing a value at line 1 column 0\n"
    );
    assert_no_accounts(&node);
}

#[test]
fn account_add_names_the_claude_json_that_is_not_an_object() {
    let node = Node::new();
    fake_claude(
        &node,
        &format!("{LOG_IN}\necho '[]' > \"$CLAUDE_CONFIG_DIR/.claude.json\""),
    );
    let _daemon = node.start_daemon();

    let stderr = stderr_of_failure(&account_add(&node, "personal").output().unwrap());

    let config = accounts_dir(&node).join("personal/claude/.claude.json");
    assert_eq!(
        stderr,
        format!(
            "error: invalid {}: invalid type: sequence, expected a map at line 1 column 0\n",
            config.display()
        )
    );
    assert_no_accounts(&node);
}
