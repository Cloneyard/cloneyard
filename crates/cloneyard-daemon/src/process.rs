use std::collections::HashMap;
use std::io;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use cloneyard_api::clone::{Handle, State};
use cloneyard_provider_claude::headless::{self, Event, Outcome};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWriteExt as _, BufReader};
use tokio::process::Child;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use ulid::Ulid;

use crate::node::Node;

/// How long a clone's processes have after SIGTERM before they get SIGKILL.
pub(crate) const GRACE: Duration = Duration::from_secs(5);

/// The processes of the clones this daemon runs, by clone id.
#[derive(Default)]
pub(crate) struct Processes {
    running: Mutex<HashMap<Ulid, Running>>,
}

struct Running {
    group: Pid,
    /// The state the clone gets once its process ends, set when the daemon ends it.
    ending: Option<(State, Option<String>)>,
    ended: watch::Receiver<bool>,
}

impl Processes {
    /// Ends the process group of clone `id` with SIGTERM, and SIGKILL after `grace`, and waits until the clone is `stopped`; `false` when the daemon runs no process for it.
    pub(crate) async fn stop(&self, id: Ulid, grace: Duration) -> bool {
        let Some(ending) = self.begin_end(id, State::Stopped, None) else {
            return false;
        };
        signal(ending.0, Signal::TERM);
        finish(ending, Instant::now() + grace).await;
        true
    }

    /// Ends the process groups of all clones the way `stop` does, with `error` and `the daemon stopped` as their state.
    pub(crate) async fn shutdown(&self, grace: Duration) {
        let ids: Vec<Ulid> = self.lock().keys().copied().collect();
        let endings: Vec<_> = ids
            .into_iter()
            .filter_map(|id| {
                self.begin_end(id, State::Error, Some("the daemon stopped".to_owned()))
            })
            .collect();
        for (group, _) in &endings {
            signal(*group, Signal::TERM);
        }
        let deadline = Instant::now() + grace;
        for ending in endings {
            finish(ending, deadline).await;
        }
    }

    fn begin_end(
        &self,
        id: Ulid,
        state: State,
        reason: Option<String>,
    ) -> Option<(Pid, watch::Receiver<bool>)> {
        self.lock().get_mut(&id).map(|running| {
            running.ending.get_or_insert((state, reason));
            (running.group, running.ended.clone())
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Ulid, Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Waits until the clone has ended, sending SIGKILL to its group at `deadline`.
async fn finish((group, mut ended): (Pid, watch::Receiver<bool>), deadline: Instant) {
    if tokio::time::timeout_at(deadline, ended.wait_for(|ended| *ended))
        .await
        .is_err()
    {
        signal(group, Signal::KILL);
        let _ = ended.wait_for(|ended| *ended).await;
    }
}

fn signal(group: Pid, signal: Signal) {
    match kill_process_group(group, signal) {
        Ok(()) | Err(Errno::SRCH) => {}
        Err(error) => {
            tracing::warn!(group = group.as_raw_nonzero(), %error, "cannot signal a clone");
        }
    }
}

/// Watches `child`, the process of clone `id`, which knows of sessions `sessions`: sends it `prompt`, records the sessions it starts, closes its stdin after its first `result` and records the clone's state.
pub(crate) fn watch(
    node: &Arc<Node>,
    id: Ulid,
    handle: Handle,
    mut child: Child,
    prompt: String,
    sessions: Vec<String>,
) -> JoinHandle<()> {
    let (done, ended) = watch::channel(false);
    if let Some(group) = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(Pid::from_raw)
    {
        node.processes.lock().insert(
            id,
            Running {
                group,
                ending: None,
                ended,
            },
        );
    }
    let node = Arc::clone(node);
    tokio::spawn(async move {
        let (result, stderr) = follow(&node, id, handle, &mut child, prompt, sessions).await;
        let status = child.wait().await;
        if result.is_none() {
            let ending = node
                .processes
                .lock()
                .get_mut(&id)
                .and_then(|running| running.ending.take());
            let (state, reason) = ending
                .unwrap_or_else(|| (State::Error, Some(cut_short(status, stderr.as_deref()))));
            record(&node, id, handle, state, reason).await;
        }
        node.tokens.revoke(id);
        node.spools.unwatch(&node.store, id).await;
        node.processes.lock().remove(&id);
        // The node goes first, so the daemon can close the database once every clone has ended.
        drop(node);
        let _ = done.send(true);
    })
}

/// Reads the clone's output until it closes, and gives the state of its first `result` and the last line of its stderr.
async fn follow(
    node: &Node,
    id: Ulid,
    handle: Handle,
    child: &mut Child,
    prompt: String,
    mut sessions: Vec<String>,
) -> (Option<State>, Option<String>) {
    let mut stdin = child.stdin.take();
    if let Some(input) = stdin.as_mut()
        && let Err(error) = input
            .write_all(headless::user_message(&prompt).as_bytes())
            .await
    {
        tracing::debug!(%handle, %error, "cannot send the prompt");
        stdin = None;
    }
    let stderr = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(log(handle, stderr)));
    let mut result = None;
    if let Some(stdout) = child.stdout.take() {
        let mut stdout = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match stdout.read_until(b'\n', &mut line).await {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%handle, %error, "cannot read the output of a clone");
                    break;
                }
            }
            match headless::parse(&String::from_utf8_lossy(&line)) {
                Some(Event::Init { session_id }) if !sessions.contains(&session_id) => {
                    if let Err(error) = node.store.add_session(id, &session_id).await {
                        tracing::error!(%handle, error = format!("{error:#}"), "cannot record a session");
                    }
                    sessions.push(session_id);
                }
                Some(Event::Result(outcome)) if result.is_none() => {
                    let (state, reason) = match outcome {
                        Outcome::Done => (State::Done, None),
                        Outcome::Error { reason } => (State::Error, Some(reason)),
                    };
                    record(node, id, handle, state, reason).await;
                    result = Some(state);
                    stdin = None;
                }
                _ => {}
            }
        }
    }
    drop(stdin);
    let last = match stderr {
        Some(stderr) => stderr.await.ok().flatten(),
        None => None,
    };
    (result, last)
}

/// Logs each line of stderr at debug level, and gives the last one that isn't blank.
async fn log(handle: Handle, stderr: impl AsyncRead + Unpin) -> Option<String> {
    let mut stderr = BufReader::new(stderr);
    let mut line = Vec::new();
    let mut last = None;
    while matches!(stderr.read_until(b'\n', &mut line).await, Ok(1..)) {
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        tracing::debug!(%handle, stderr = text);
        if !text.is_empty() {
            last = Some(text.to_owned());
        }
        line.clear();
    }
    last
}

fn cut_short(status: io::Result<ExitStatus>, stderr: Option<&str>) -> String {
    let ended = match status {
        Ok(status) => format!("claude ended without a result ({status})"),
        Err(error) => format!("cannot wait for claude: {error}"),
    };
    match stderr {
        Some(line) => format!("{ended}: {line}"),
        None => ended,
    }
}

async fn record(node: &Node, id: Ulid, handle: Handle, state: State, reason: Option<String>) {
    tracing::info!(%handle, %state, reason, "clone state");
    if let Err(error) = node.store.set_clone_state(id, state, reason).await {
        tracing::error!(%handle, error = format!("{error:#}"), "cannot record the state of a clone");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Stdio;
    use std::time::Duration;

    use cloneyard_api::account::Provider;
    use cloneyard_api::clone::{Clone, Mode, State};
    use cloneyard_provider_claude::headless;
    use cloneyard_store::NewClone;
    use tempfile::TempDir;
    use tokio::process::{Child, Command};
    use tokio::task::JoinHandle;
    use ulid::Ulid;

    use super::watch;
    use crate::node::Node;
    use crate::router::tests::open_node;

    const INIT: &str = r#"echo '{"type":"system","subtype":"init","session_id":"s1"}'"#;
    const DONE: &str =
        r#"echo '{"type":"result","subtype":"success","is_error":false,"result":"pong"}'"#;

    /// A clone on the node's one project and account, which it adds when they are missing.
    async fn new_clone(node: &Node) -> NewClone {
        let projects = node.store.projects(node.id).await.unwrap();
        let project = match projects.into_iter().next() {
            Some(project) => project,
            None => node
                .store
                .add_project(node.id, "app", "app", "/code/app")
                .await
                .unwrap()
                .unwrap(),
        };
        let accounts = node.store.accounts(node.id).await.unwrap();
        let account = match accounts.into_iter().next() {
            Some(account) => account,
            None => node
                .store
                .add_account(node.id, "personal", Provider::ClaudeCode, "/data/personal")
                .await
                .unwrap()
                .unwrap(),
        };
        NewClone {
            id: Ulid::generate(),
            project_id: project.id,
            repo_id: project.repos[0].id,
            account_id: account.id,
            mode: Mode::Headless,
            checkout: "/data/clones/x/checkout".to_owned(),
            session: "s1".to_owned(),
            unsandboxed: false,
        }
    }

    async fn add_clone(node: &Node) -> Clone {
        let clone = new_clone(node).await;
        node.store
            .add_clone(node.id, clone, |handle| format!("branch-{handle}"))
            .await
            .unwrap()
    }

    fn sh(dir: &TempDir, script: &str) -> Child {
        Command::new("sh")
            .arg("-c")
            .arg(script)
            .env("DIR", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap()
    }

    async fn run(dir: &TempDir, script: &str) -> Clone {
        let node = open_node(dir).await;
        let clone = add_clone(&node).await;
        let child = sh(dir, script);
        let task = watch(
            &node,
            clone.id,
            clone.handle,
            child,
            "Say pong".to_owned(),
            vec!["s1".to_owned()],
        );
        ended(task).await;
        shown(&node, &clone).await
    }

    async fn ended(task: JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the clone did not end")
            .unwrap();
    }

    async fn shown(node: &Node, clone: &Clone) -> Clone {
        node.store
            .clone_by_handle(node.id, clone.handle)
            .await
            .unwrap()
            .unwrap()
    }

    async fn wait_for(path: &Path) {
        for _ in 0..1000 {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {}", path.display());
    }

    #[tokio::test]
    async fn a_result_without_an_error_ends_the_clone_done_after_its_prompt_and_closes_its_stdin() {
        let dir = TempDir::new().unwrap();
        let script = format!(
            "IFS= read -r prompt; printf '%s\\n' \"$prompt\" > \"$DIR/prompt\"\n{INIT}\n{DONE}\n\
             cat > /dev/null; touch \"$DIR/eof\""
        );

        let clone = run(&dir, &script).await;

        let prompt = std::fs::read_to_string(dir.path().join("prompt")).unwrap();
        assert_eq!((clone.state, clone.reason), (State::Done, None));
        assert_eq!(prompt, headless::user_message("Say pong"));
        assert!(dir.path().join("eof").exists());
    }

    #[tokio::test]
    async fn a_result_with_is_error_ends_the_clone_error_with_its_reason_even_with_subtype_success()
    {
        let dir = TempDir::new().unwrap();
        let script = format!(
            "{INIT}\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":true,\
             \"result\":\"Failed to refresh OAuth token\"}}'"
        );

        let clone = run(&dir, &script).await;

        assert_eq!(
            (clone.state, clone.reason.as_deref()),
            (State::Error, Some("Failed to refresh OAuth token"))
        );
    }

    #[tokio::test]
    async fn an_init_with_a_new_session_id_adds_a_session_and_a_known_one_does_not() {
        let dir = TempDir::new().unwrap();
        let second = r#"echo '{"type":"system","subtype":"init","session_id":"s2"}'"#;
        let script = format!("{INIT}\n{second}\n{second}\n{DONE}");

        let clone = run(&dir, &script).await;

        let sessions: Vec<&str> = clone
            .sessions
            .iter()
            .map(|session| session.provider_id.as_str())
            .collect();
        assert_eq!(sessions, ["s1", "s2"]);
    }

    #[tokio::test]
    async fn a_clone_that_ends_without_a_result_ends_error_with_its_status_and_last_line_of_stderr()
    {
        let dir = TempDir::new().unwrap();
        let script = format!(
            "{INIT}\necho 'not json'\necho first >&2\necho 'Not logged in' >&2\necho >&2\nexit 3"
        );

        let clone = run(&dir, &script).await;

        assert_eq!(
            (clone.state, clone.reason.as_deref()),
            (
                State::Error,
                Some("claude ended without a result (exit status: 3): Not logged in")
            )
        );
    }

    #[tokio::test]
    async fn a_stopped_clone_ends_stopped_and_takes_its_children_with_it() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = add_clone(&node).await;
        let child = sh(
            &dir,
            &format!("{INIT}\nsleep 1000 & echo $! > \"$DIR/child\"; wait"),
        );
        let task = watch(
            &node,
            clone.id,
            clone.handle,
            child,
            String::new(),
            Vec::new(),
        );
        wait_for(&dir.path().join("child")).await;
        let pid = std::fs::read_to_string(dir.path().join("child")).unwrap();

        let stopped = node.processes.stop(clone.id, Duration::from_secs(5)).await;

        ended(task).await;
        assert!(stopped);
        assert_eq!(shown(&node, &clone).await.state, State::Stopped);
        wait_for_gone(&Path::new("/proc").join(pid.trim())).await;
    }

    #[tokio::test]
    async fn a_clone_that_ignores_sigterm_gets_sigkill_after_the_grace() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = add_clone(&node).await;
        let child = sh(&dir, "trap '' TERM; touch \"$DIR/ready\"; sleep 1000");
        let task = watch(
            &node,
            clone.id,
            clone.handle,
            child,
            String::new(),
            Vec::new(),
        );
        wait_for(&dir.path().join("ready")).await;

        let stopped = node
            .processes
            .stop(clone.id, Duration::from_millis(100))
            .await;

        ended(task).await;
        assert!(stopped);
        assert_eq!(shown(&node, &clone).await.state, State::Stopped);
    }

    #[tokio::test]
    async fn shutdown_ends_every_clone_with_error_and_waits_for_them() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let first = add_clone(&node).await;
        let second = node
            .store
            .add_clone(
                node.id,
                NewClone {
                    id: Ulid::generate(),
                    session: "s2".to_owned(),
                    ..new_clone(&node).await
                },
                |handle| format!("branch-{handle}"),
            )
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for (clone, ready) in [(&first, "first"), (&second, "second")] {
            let child = sh(&dir, &format!("touch \"$DIR/{ready}\"; sleep 1000"));
            tasks.push(watch(
                &node,
                clone.id,
                clone.handle,
                child,
                String::new(),
                Vec::new(),
            ));
            wait_for(&dir.path().join(ready)).await;
        }

        node.processes.shutdown(Duration::from_secs(5)).await;

        for task in tasks {
            ended(task).await;
        }
        for clone in [&first, &second] {
            let shown = shown(&node, clone).await;
            assert_eq!(
                (shown.state, shown.reason.as_deref()),
                (State::Error, Some("the daemon stopped"))
            );
        }
    }

    #[tokio::test]
    async fn stopping_a_clone_without_a_process_does_nothing() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;

        let stopped = node
            .processes
            .stop(Ulid::generate(), Duration::from_millis(100))
            .await;

        assert!(!stopped);
    }

    async fn wait_for_gone(path: &Path) {
        for _ in 0..1000 {
            if !path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{} is still there", path.display());
    }
}
