use std::env;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{
    Daemon, LOG_IN, Node, account_add, bin_path, claude_dir, fake_claude, project_add,
    stderr_of_failure, stdout_of_success, wait_until,
};

mod common;

/// Records what `claude` got in `clones/<session id>/` in its account's directory, which it can write, then starts its session.
const RECORD: &str = r#"out="$CLAUDE_CONFIG_DIR/clones/$session"
mkdir -p "$out"
printf '%s\n' "$@" > "$out/args"
printf '%s' "$CLAUDE_CONFIG_DIR" > "$out/config-dir"
printf '%s' "${ANTHROPIC_API_KEY-unset}" > "$out/api-key"
printf '%s' "${CLONEYARD_SPOOL-unset}" > "$out/spool"
pwd > "$out/cwd"
git branch --show-current > "$out/branch"
echo $$ > "$out/pid"
IFS= read -r prompt
printf '%s\n' "$prompt" > "$out/prompt"
printf '{"type":"system","subtype":"init","session_id":"%s"}\n' "$session""#;

const WAIT_FOR_GO: &str = r#"while [ ! -e "$HOME/go" ]; do sleep 0.02; done"#;

/// `hook <event> <fields>` runs `cy hook` as Claude Code does, with the session id and `fields` in the JSON on stdin.
const HOOK: &str =
    r#"hook() { printf '{"session_id":"%s"%s}' "$session" "$2" | "$CY" hook "$1"; }"#;

const DONE: &str = r#"echo '{"type":"result","subtype":"success","is_error":false,"result":"pong"}'
cat > /dev/null"#;

/// A git repo at `path` with one commit, given as its canonical path.
fn repo_with_commit(path: &Path) -> PathBuf {
    fs::create_dir_all(path).unwrap();
    git(path, &["init", "--quiet", "--initial-branch=main"]);
    fs::write(path.join("README.md"), "app\n").unwrap();
    git(path, &["add", "README.md"]);
    git(path, &["commit", "--quiet", "-m", "first"]);
    fs::canonicalize(path).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// Replaces the fake `claude` with one that runs `body` for `claude -p`, with `$session` set to its `--session-id`.
fn fake_headless(node: &Node, body: &str) {
    let claude = node.home.path().join("bin/claude");
    fs::write(
        &claude,
        format!(
            r#"#!/bin/sh
[ "$1" = "-p" ] || exit 2
session=
previous=
for arg in "$@"; do
    [ "$previous" = "--session-id" ] && session="$arg"
    previous="$arg"
done
{body}
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A daemon that finds the fake `claude` first on its `PATH`, which can run `cy` as `$CY`.
fn start_daemon(node: &Node) -> Daemon {
    let mut daemon = node.cy("daemon");
    daemon
        .env("PATH", bin_path(node))
        .env("CY", env!("CARGO_BIN_EXE_cy"));
    node.spawn(daemon)
}

/// A node with project `app` and account `personal`, and its daemon.
fn node_with_project_and_account() -> (Node, PathBuf, Daemon) {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let repo = repo_with_commit(&node.home.path().join("app"));
    let daemon = start_daemon(&node);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());
    (node, repo, daemon)
}

fn run(node: &Node, prompt: &str) -> std::process::Output {
    node.cy("clone")
        .args(["run", "--project", "app", "--account", "personal", prompt])
        .output()
        .unwrap()
}

fn clones(node: &Node) -> String {
    stdout_of_success(&node.cy("clones").output().unwrap())
}

/// The lines of `cy clone show <handle>` as keys and values.
fn show(node: &Node, handle: &str) -> Vec<(String, String)> {
    let output = node.cy("clone").args(["show", handle]).output().unwrap();
    stdout_of_success(&output)
        .lines()
        .map(|line| {
            let (key, value) = line.split_once("  ").unwrap();
            (key.trim().to_owned(), value.trim().to_owned())
        })
        .collect()
}

fn field(lines: &[(String, String)], key: &str) -> String {
    let (_, value) = lines
        .iter()
        .find(|(name, _)| name == key)
        .unwrap_or_else(|| panic!("no {key} in {lines:?}"));
    value.clone()
}

fn wait_for_state(node: &Node, line: &str) {
    wait_until(line, || clones(node).lines().any(|shown| shown == line));
}

fn recorded(node: &Node, session: &str, name: &str) -> String {
    let path = claude_dir(node, "personal")
        .join("clones")
        .join(session)
        .join(name);
    wait_until(&format!("{}", path.display()), || path.exists());
    fs::read_to_string(path).unwrap()
}

fn wait_for_gone(pid: &str) {
    let proc = Path::new("/proc").join(pid.trim());
    wait_until(&format!("process {pid} to end"), || !proc.exists());
}

#[test]
fn clone_run_prints_the_handle_and_clones_shows_it_working_then_done_after_a_restart() {
    let (node, _repo, daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{WAIT_FOR_GO}\n{DONE}"));

    let stdout = stdout_of_success(&run(&node, "Say pong"));

    assert_eq!(stdout, "CT-0001\n");
    wait_for_state(&node, "CT-0001  app  personal  headless  working");
    fs::write(node.home.path().join("go"), "").unwrap();
    wait_for_state(&node, "CT-0001  app  personal  headless  done");
    daemon.terminate();
    let _daemon = start_daemon(&node);
    assert_eq!(clones(&node), "CT-0001  app  personal  headless  done\n");
    assert_eq!(stdout_of_success(&run(&node, "Again")), "CT-0002\n");
}

#[test]
fn the_clone_runs_claude_headless_on_its_session_and_account_in_its_checkout() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let repo = repo_with_commit(&node.home.path().join("app"));
    let mut daemon = node.cy("daemon");
    daemon
        .env("PATH", bin_path(&node))
        .env("ANTHROPIC_API_KEY", "sk-test");
    let _daemon = node.spawn(daemon);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());
    fake_headless(&node, &format!("{RECORD}\n{DONE}"));

    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let shown = show(&node, "ct-1");
    let session = field(&shown, "session");
    let prompt: serde_json::Value =
        serde_json::from_str(&recorded(&node, &session, "prompt")).unwrap();
    let cy = fs::canonicalize(env!("CARGO_BIN_EXE_cy")).unwrap();
    assert_eq!(
        recorded(&node, &session, "args"),
        format!(
            "-p\n--input-format\nstream-json\n--output-format\nstream-json\n--verbose\n\
             --session-id\n{session}\n--permission-prompts\nnone\n--settings\n{}\n",
            cloneyard_provider_claude::settings(cy.to_str().unwrap(), true)
        )
    );
    assert_eq!(
        recorded(&node, &session, "config-dir"),
        claude_dir(&node, "personal").to_str().unwrap()
    );
    assert_eq!(recorded(&node, &session, "api-key"), "unset");
    assert_eq!(
        PathBuf::from(recorded(&node, &session, "spool")),
        Path::new(&field(&shown, "checkout")).with_file_name("spool")
    );
    assert_eq!(
        recorded(&node, &session, "cwd").trim_end(),
        field(&shown, "checkout")
    );
    assert_eq!(recorded(&node, &session, "branch"), "cloneyard/ct-0001\n");
    assert_eq!(field(&shown, "branch"), "cloneyard/ct-0001");
    assert_eq!(
        prompt["message"]["content"][0]["text"],
        serde_json::json!("Say pong")
    );
}

#[test]
fn clone_show_lists_the_clone_with_its_checkout_and_session() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{DONE}"));
    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let shown = show(&node, "CT-0001");

    let keys: Vec<&str> = shown.iter().map(|(key, _)| key.as_str()).collect();
    let checkout = PathBuf::from(field(&shown, "checkout"));
    assert_eq!(
        keys,
        [
            "handle", "id", "project", "account", "mode", "state", "sandbox", "branch", "checkout",
            "session"
        ]
    );
    assert_eq!(field(&shown, "handle"), "CT-0001");
    assert_eq!(field(&shown, "state"), "done");
    assert_eq!(field(&shown, "sandbox"), "on");
    assert_eq!(
        checkout,
        fs::canonicalize(node.home.path().join("data/cloneyard/clones"))
            .unwrap()
            .join(field(&shown, "id"))
            .join("checkout")
    );
}

#[test]
fn a_commit_the_clone_makes_stays_in_its_checkout() {
    let (node, repo, _daemon) = node_with_project_and_account();
    let before = (
        git(&repo, &["for-each-ref"]),
        git(&repo, &["rev-parse", "HEAD"]),
    );
    fake_headless(
        &node,
        &format!(
            "{RECORD}\necho change > README.md\n\
             git -c user.name=Clone -c user.email=clone@example.com commit --quiet -am 'by the clone'\n\
             {DONE}"
        ),
    );

    stdout_of_success(&run(&node, "Commit something"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let checkout = PathBuf::from(field(&show(&node, "CT-0001"), "checkout"));
    let commit = git(&checkout, &["rev-parse", "HEAD"]);
    let in_repo = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["cat-file", "-e", commit.trim()])
        .status()
        .unwrap();
    assert_ne!(commit, before.1);
    assert_eq!(
        (
            git(&repo, &["for-each-ref"]),
            git(&repo, &["rev-parse", "HEAD"])
        ),
        before
    );
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(fs::read_to_string(repo.join("README.md")).unwrap(), "app\n");
    assert!(!in_repo.success());
}

#[test]
fn a_clone_whose_result_is_an_error_or_that_ends_without_one_is_error_with_the_reason() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(
        &node,
        &format!(
            "{RECORD}\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\
             \"terminal_reason\":\"api_error\",\"result\":\"Failed to refresh OAuth token\"}}'"
        ),
    );
    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0001  app  personal  headless  error");
    fake_headless(&node, "echo 'Not logged in' >&2\nexit 1");

    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0002  app  personal  headless  error");

    assert_eq!(
        field(&show(&node, "CT-0001"), "reason"),
        "api_error: Failed to refresh OAuth token"
    );
    assert_eq!(
        field(&show(&node, "CT-0002"), "reason"),
        "claude ended without a result (exit status: 1): Not logged in"
    );
}

#[test]
fn clone_stop_ends_a_working_clone_with_its_children_and_refuses_one_that_is_not_working() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(
        &node,
        &format!("{RECORD}\nsleep 1000 & echo $! > \"$out/child\"\nwait"),
    );
    stdout_of_success(&run(&node, "Say pong"));
    let session = field(&show(&node, "CT-0001"), "session");
    let child = recorded(&node, &session, "child");

    let stopped = stdout_of_success(&node.cy("clone").args(["stop", "CT-0001"]).output().unwrap());
    let again = stderr_of_failure(&node.cy("clone").args(["stop", "CT-0001"]).output().unwrap());

    assert_eq!(stopped, "stopped CT-0001\n");
    assert_eq!(clones(&node), "CT-0001  app  personal  headless  stopped\n");
    assert_eq!(again, "error: clone CT-0001 is stopped, not running\n");
    wait_for_gone(&child);
    wait_for_gone(&recorded(&node, &session, "pid"));
}

#[test]
fn the_daemon_ends_its_clones_on_sigterm_and_they_are_error_after_the_restart() {
    let (node, _repo, daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{WAIT_FOR_GO}\n{DONE}"));
    stdout_of_success(&run(&node, "Say pong"));
    let session = field(&show(&node, "CT-0001"), "session");
    let pid = recorded(&node, &session, "pid");

    let status = daemon.terminate();
    let _daemon = start_daemon(&node);

    assert!(status.success(), "{status:?}");
    wait_for_gone(&pid);
    assert_eq!(clones(&node), "CT-0001  app  personal  headless  error\n");
    assert_eq!(
        field(&show(&node, "CT-0001"), "reason"),
        "the daemon stopped"
    );
}

#[test]
fn a_clone_lost_when_the_daemon_is_killed_is_error_once_the_daemon_starts_again() {
    let (node, _repo, mut daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{WAIT_FOR_GO}\n{DONE}"));
    stdout_of_success(&run(&node, "Say pong"));
    let session = field(&show(&node, "CT-0001"), "session");
    let pid = recorded(&node, &session, "pid");

    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    let _daemon = start_daemon(&node);

    assert_eq!(clones(&node), "CT-0001  app  personal  headless  error\n");
    assert_eq!(
        field(&show(&node, "CT-0001"), "reason"),
        "lost while the daemon was down"
    );
    fs::write(node.home.path().join("go"), "").unwrap();
    wait_for_gone(&pid);
}

#[test]
fn clone_commands_name_the_project_account_or_clone_they_do_not_know() {
    let (node, _repo, _daemon) = node_with_project_and_account();

    let messages: Vec<String> = [
        node.cy("clone")
            .args(["run", "--project", "web", "--account", "personal", "Hi"])
            .output(),
        node.cy("clone")
            .args(["run", "--project", "app", "--account", "work", "Hi"])
            .output(),
        node.cy("clone").args(["show", "CT-0009"]).output(),
        node.cy("clone").args(["stop", "ct-9"]).output(),
    ]
    .map(|output| stderr_of_failure(&output.unwrap()))
    .into();

    assert_eq!(
        messages,
        [
            "error: no project web\n",
            "error: no account work\n",
            "error: no clone CT-0009\n",
            "error: no clone CT-0009\n",
        ]
    );
    assert_eq!(clones(&node), "");
}

#[test]
fn clone_run_prints_the_handle_and_fails_with_the_reason_when_claude_does_not_start() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let repo = repo_with_commit(&node.home.path().join("app"));
    let only_git = node.home.path().join("only-git");
    fs::create_dir(&only_git).unwrap();
    let git = env::split_paths(&env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|git| git.exists())
        .unwrap();
    symlink(git, only_git.join("git")).unwrap();
    let mut daemon = node.cy("daemon");
    daemon.env("PATH", &only_git);
    let _daemon = node.spawn(daemon);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());

    let output = run(&node, "Say pong");

    assert_eq!(
        String::from_utf8(output.stdout.clone()).unwrap(),
        "CT-0001\n"
    );
    assert_eq!(
        stderr_of_failure(&output),
        "error: clone CT-0001 did not start: cannot start claude: \
         No such file or directory (os error 2)\n"
    );
    assert_eq!(clones(&node), "CT-0001  app  personal  headless  error\n");
}

#[test]
fn a_clone_cannot_read_the_repos_env_or_an_ssh_key_but_runs_a_build_and_commits_in_its_checkout() {
    let (node, repo, _daemon) = node_with_project_and_account();
    fs::write(repo.join(".env"), "TOKEN=secret\n").unwrap();
    let ssh = node.home.path().join(".ssh");
    fs::create_dir(&ssh).unwrap();
    fs::write(ssh.join("id_ed25519"), "key\n").unwrap();
    fake_headless(
        &node,
        &format!(
            r#"{RECORD}
repo=$(dirname "$(dirname "$(cat .git/objects/info/alternates)")")
cat "$repo/.env" > "$out/env" 2> "$out/env-error"
cat "$HOME/.ssh/id_ed25519" > "$out/key" 2> "$out/key-error"
echo 'echo built > built.txt' > build.sh
sh build.sh
git add build.sh built.txt
git -c user.name=Clone -c user.email=clone@example.com commit --quiet -m build
git log -1 --format=%s > "$out/commit"
{DONE}"#
        ),
    );

    stdout_of_success(&run(&node, "Print the secrets"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let session = field(&show(&node, "CT-0001"), "session");
    assert_eq!(recorded(&node, &session, "commit"), "build\n");
    for (read, error) in [("env", "env-error"), ("key", "key-error")] {
        assert_eq!(recorded(&node, &session, read), "");
        let error = recorded(&node, &session, error);
        assert!(error.contains("Permission denied"), "{error}");
    }
}

#[test]
fn a_clone_cannot_write_outside_its_directory_reach_the_daemon_socket_or_push_to_the_repo() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let repo = repo_with_commit(&node.home.path().join("app"));
    let mut daemon = node.cy("daemon");
    daemon
        .env("PATH", bin_path(&node))
        .env("CY", env!("CARGO_BIN_EXE_cy"));
    let _daemon = node.spawn(daemon);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());
    let refs = git(&repo, &["for-each-ref"]);
    fake_headless(
        &node,
        &format!(
            r#"{RECORD}
{{ echo outside > "$HOME/outside"; }} 2> "$out/write-error"
"$CY" ping 2> "$out/ping-error"
git push --quiet origin HEAD:refs/heads/from-clone 2> "$out/push-error"
echo $? > "$out/push"
{DONE}"#
        ),
    );

    stdout_of_success(&run(&node, "Break out"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let session = field(&show(&node, "CT-0001"), "session");
    let ping = recorded(&node, &session, "ping-error");
    assert_ne!(recorded(&node, &session, "push"), "0\n");
    assert!(!node.home.path().join("outside").exists());
    assert!(
        recorded(&node, &session, "write-error").contains("Permission denied"),
        "{}",
        recorded(&node, &session, "write-error")
    );
    assert!(ping.contains("Permission denied"), "{ping}");
    assert_eq!(git(&repo, &["for-each-ref"]), refs);
}

#[test]
fn with_unsandboxed_in_node_toml_a_clone_runs_without_the_sandbox_and_clones_says_so() {
    let node = Node::new();
    fake_claude(&node, LOG_IN);
    let config = node.home.path().join("config/cloneyard");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("node.toml"), "unsandboxed = true\n").unwrap();
    let repo = repo_with_commit(&node.home.path().join("app"));
    let _daemon = start_daemon(&node);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());
    fake_headless(
        &node,
        &format!("{RECORD}\necho outside > \"$HOME/outside\"\n{DONE}"),
    );

    stdout_of_success(&run(&node, "Say pong"));

    wait_for_state(&node, "CT-0001  app  personal  headless  done  unsandboxed");
    assert!(node.home.path().join("outside").exists());
    assert_eq!(field(&show(&node, "CT-0001"), "sandbox"), "off");
}

#[test]
fn a_clone_of_a_repo_in_tmp_does_not_start_since_tmp_is_writable_and_names_what_it_cannot_hide() {
    let node = Node {
        home: tempfile::TempDir::new_in("/tmp").unwrap(),
    };
    fake_claude(&node, LOG_IN);
    let home = fs::canonicalize(node.home.path()).unwrap();
    let repo = repo_with_commit(&home.join("app"));
    let _daemon = start_daemon(&node);
    stdout_of_success(&project_add(&node, &repo));
    stdout_of_success(&account_add(&node, "personal").output().unwrap());

    let output = run(&node, "Say pong");

    assert_eq!(
        stderr_of_failure(&output),
        format!(
            "error: clone CT-0001 did not start: cannot sandbox the clone: \
             cannot hide {}/.ssh inside /tmp, which the clone can read\n",
            home.display()
        )
    );
}

#[test]
fn a_clones_state_follows_its_hooks_until_its_result() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(
        &node,
        &format!(
            r#"{RECORD}
{HOOK}
hook Notification ',"notification_type":"permission_prompt"'
while [ ! -e "$HOME/answered" ]; do sleep 0.02; done
hook PostToolUse ',"tool_name":"Bash"'
hook Stop
{WAIT_FOR_GO}
{DONE}"#
        ),
    );

    stdout_of_success(&run(&node, "Run the tests"));

    wait_for_state(&node, "CT-0001  app  personal  headless  blocked");
    fs::write(node.home.path().join("answered"), "").unwrap();
    wait_for_state(&node, "CT-0001  app  personal  headless  idle");
    fs::write(node.home.path().join("go"), "").unwrap();
    wait_for_state(&node, "CT-0001  app  personal  headless  done");
}

#[test]
fn a_clone_cannot_write_into_the_spool_of_another_clone() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{DONE}"));
    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");
    let first = Path::new(&field(&show(&node, "CT-0001"), "checkout")).with_file_name("spool");
    let forged = first.join("00000000000000000001-1-Stop.json");
    fake_headless(
        &node,
        &format!(
            r#"{RECORD}
{{ echo '{{}}' > '{}'; }} 2> "$out/spool-error"
{DONE}"#,
            forged.display()
        ),
    );

    stdout_of_success(&run(&node, "Forge an event"));
    wait_for_state(&node, "CT-0002  app  personal  headless  done");

    let session = field(&show(&node, "CT-0002"), "session");
    let error = recorded(&node, &session, "spool-error");
    assert!(error.contains("Permission denied"), "{error}");
    assert!(!forged.exists());
}

/// The hooks and tools in the `event` lines of `cy clone show <handle>`, without their ages.
fn events(node: &Node, handle: &str) -> Vec<String> {
    show(node, handle)
        .into_iter()
        .filter(|(key, _)| key == "event")
        .map(|(_, value)| {
            let (label, age) = value.rsplit_once("  ").unwrap();
            assert!(age.ends_with(" ago"), "{value}");
            label.trim().to_owned()
        })
        .collect()
}

#[test]
fn clone_show_lists_a_clones_tool_calls_among_its_last_events() {
    let (node, _repo, _daemon) = node_with_project_and_account();
    fake_headless(
        &node,
        &format!(
            r#"{RECORD}
{HOOK}
hook SessionStart ',"source":"startup"'
hook UserPromptSubmit ',"prompt":"Run the tests"'
hook PreToolUse ',"tool_name":"Bash","tool_input":{{"command":"cargo test"}}'
hook PostToolUse ',"tool_name":"Bash","tool_response":{{"stdout":"ok"}}'
hook Stop
{DONE}"#
        ),
    );

    stdout_of_success(&run(&node, "Run the tests"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");

    let expected = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse Bash",
        "PostToolUse Bash",
        "Stop",
    ];
    wait_until("the hook events", || events(&node, "CT-0001") == expected);
}

#[test]
fn events_written_while_the_daemon_is_stopped_show_up_after_it_starts_and_leave_the_spool_empty() {
    let (node, _repo, daemon) = node_with_project_and_account();
    fake_headless(&node, &format!("{RECORD}\n{DONE}"));
    stdout_of_success(&run(&node, "Say pong"));
    wait_for_state(&node, "CT-0001  app  personal  headless  done");
    let shown = show(&node, "CT-0001");
    let session = field(&shown, "session");
    let spool = Path::new(&field(&shown, "checkout")).with_file_name("spool");
    daemon.terminate();

    for (event, fields) in [
        ("PreToolUse", r#","tool_name":"Read""#),
        ("PostToolUse", r#","tool_name":"Read""#),
        ("SessionEnd", r#","reason":"other""#),
    ] {
        let payload = format!(r#"{{"session_id":"{session}"{fields}}}"#);
        let mut hook = node.cy("hook");
        hook.arg(event)
            .env("CLONEYARD_SPOOL", &spool)
            .stdin(Stdio::piped());
        let mut child = hook.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }
    let _daemon = start_daemon(&node);

    assert_eq!(
        events(&node, "CT-0001"),
        ["PreToolUse Read", "PostToolUse Read", "SessionEnd"]
    );
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 0);
}
