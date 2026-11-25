use std::env;
use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context as _, bail};
use cloneyard_api::clone::{
    Clone, EventsError, EventsParams, EventsResult, HandleParams, ListResult, Mode, RunError,
    RunParams, ShowError, State, StopError,
};
use cloneyard_provider_claude::{self as provider, headless};
use cloneyard_store::NewClone;
use tokio::process::Command;
use ulid::Ulid;
use uuid::Uuid;

use crate::node::Node;
use crate::process::{self, GRACE};
use crate::sandbox::{self, Sandboxing};
use crate::spool;

/// Records a `working` clone, makes its checkout and starts it; a clone whose checkout or start fails ends `error`, with the reason.
pub(crate) async fn run(
    node: &Arc<Node>,
    params: RunParams,
) -> anyhow::Result<Result<Clone, RunError>> {
    let RunParams {
        project,
        account,
        prompt,
    } = params;
    let Some(project) = node
        .store
        .projects(node.id)
        .await?
        .into_iter()
        .find(|known| known.name == project)
    else {
        return Ok(Err(RunError::UnknownProject { name: project }));
    };
    let Some(account) = node
        .store
        .accounts(node.id)
        .await?
        .into_iter()
        .find(|known| known.name == account)
    else {
        return Ok(Err(RunError::UnknownAccount { name: account }));
    };
    if let Sandboxing::Unavailable { config_file } = &node.sandboxing {
        return Ok(Err(RunError::NoSandbox {
            config: config_file.display().to_string(),
        }));
    }
    let repo = project
        .repos
        .first()
        .with_context(|| format!("project {} has no repo on this node", project.name))?;
    let account_dir = account
        .dir
        .as_deref()
        .with_context(|| format!("account {} has no directory on this node", account.name))?;
    let id = Ulid::generate();
    let checkout = node
        .data_dir
        .join("clones")
        .join(id.to_string())
        .join("checkout");
    let checkout = checkout
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", checkout.display()))?
        .to_owned();
    let session_id = Uuid::new_v4();
    let clone = node
        .store
        .add_clone(
            node.id,
            NewClone {
                id,
                project_id: project.id,
                repo_id: repo.id,
                account_id: account.id,
                mode: Mode::Headless,
                checkout,
                session: session_id.to_string(),
                unsandboxed: node.sandboxing == Sandboxing::Off,
            },
            |handle| format!("cloneyard/{}", handle.to_string().to_lowercase()),
        )
        .await?;
    match start(
        node,
        &clone,
        Path::new(&repo.path),
        Path::new(account_dir),
        session_id,
        prompt,
    )
    .await
    {
        Ok(()) => Ok(Ok(clone)),
        Err(reason) => {
            tracing::warn!(handle = %clone.handle, reason, "clone did not start");
            node.store
                .set_clone_state(id, State::Error, Some(reason.clone()))
                .await?;
            Ok(Ok(Clone {
                state: State::Error,
                reason: Some(reason),
                ..clone
            }))
        }
    }
}

async fn start(
    node: &Arc<Node>,
    clone: &Clone,
    repo: &Path,
    account_dir: &Path,
    session_id: Uuid,
    prompt: String,
) -> Result<(), String> {
    let checkout = Path::new(&clone.checkout);
    let clone_dir = checkout
        .parent()
        .ok_or_else(|| format!("{} has no parent", checkout.display()))?;
    make_checkout(repo, checkout, &clone.branch)
        .await
        .map_err(|error| format!("cannot create the checkout: {error:#}"))?;
    let settings = provider::settings(&node.program, !clone.unsandboxed);
    let mut claude = headless::command(account_dir, session_id, &settings);
    claude
        .current_dir(checkout)
        .env(spool::VARIABLE, spool::dir(clone_dir));
    let mut command = if clone.unsandboxed {
        claude
    } else {
        let policy = sandbox::policy(node, env::home_dir().as_deref(), clone_dir, account_dir)
            .await
            .map_err(|error| format!("cannot sandbox the clone: {error:#}"))?;
        provider::inside_sandbox(&mut claude);
        sandbox::use_caches(&mut claude, &node.data_dir);
        cloneyard_sandbox::command(Path::new(sandbox::ENTER), &policy, claude.as_std())
            .map(Command::from)
            .map_err(|error| format!("cannot start claude: {error}"))?
    };
    node.spools
        .watch(clone.id, clone_dir)
        .map_err(|error| format!("cannot set up the spool: {error:#}"))?;
    let child = match command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            node.spools.unwatch(&node.store, clone.id).await;
            return Err(format!("cannot start claude: {error}"));
        }
    };
    tracing::info!(handle = %clone.handle, checkout = clone.checkout, "clone started");
    process::watch(
        node,
        clone.id,
        clone.handle,
        child,
        prompt,
        vec![session_id.to_string()],
    );
    Ok(())
}

/// A `git clone --shared` of `repo` at `checkout`, on a new branch `branch`; git runs there only before the agent does.
async fn make_checkout(repo: &Path, checkout: &Path, branch: &str) -> anyhow::Result<()> {
    let dir = checkout
        .parent()
        .with_context(|| format!("{} has no parent", checkout.display()))?;
    if let Some(clones) = dir.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(clones)
            .with_context(|| format!("cannot create {}", clones.display()))?;
    }
    DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    git(&[
        OsStr::new("clone"),
        OsStr::new("--shared"),
        OsStr::new("--quiet"),
        OsStr::new("--"),
        repo.as_os_str(),
        checkout.as_os_str(),
    ])
    .await?;
    git(&[
        OsStr::new("-C"),
        checkout.as_os_str(),
        OsStr::new("switch"),
        OsStr::new("--quiet"),
        OsStr::new("--create"),
        OsStr::new(branch),
    ])
    .await
}

async fn git(args: &[&OsStr]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .context("cannot run git")?;
    if !output.status.success() {
        let shown: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        bail!(
            "git {} failed ({}): {}",
            shown.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub(crate) async fn list(node: &Node) -> anyhow::Result<ListResult> {
    Ok(ListResult {
        clones: node.store.clones(node.id).await?,
    })
}

pub(crate) async fn show(
    node: &Node,
    params: HandleParams,
) -> anyhow::Result<Result<Clone, ShowError>> {
    let HandleParams { handle } = params;
    Ok(node
        .store
        .clone_by_handle(node.id, handle)
        .await?
        .ok_or(ShowError::UnknownClone { handle }))
}

/// The last `limit` hook events of a clone, oldest first.
pub(crate) async fn events(
    node: &Node,
    params: EventsParams,
) -> anyhow::Result<Result<EventsResult, EventsError>> {
    let EventsParams { handle, limit } = params;
    let Some(clone) = node.store.clone_by_handle(node.id, handle).await? else {
        return Ok(Err(EventsError::UnknownClone { handle }));
    };
    Ok(Ok(EventsResult {
        events: node.store.hook_events(clone.id, limit).await?,
    }))
}

/// Ends a running clone's processes and gives it back `stopped`.
pub(crate) async fn stop(
    node: &Node,
    params: HandleParams,
) -> anyhow::Result<Result<Clone, StopError>> {
    let HandleParams { handle } = params;
    let Some(clone) = node.store.clone_by_handle(node.id, handle).await? else {
        return Ok(Err(StopError::UnknownClone { handle }));
    };
    if !clone.state.runs() {
        return Ok(Err(StopError::NotRunning {
            handle,
            state: clone.state,
        }));
    }
    if !node.processes.stop(clone.id, GRACE).await {
        bail!(
            "clone {handle} is {}, but this daemon runs no process for it",
            clone.state
        );
    }
    node.store
        .clone_by_handle(node.id, handle)
        .await?
        .with_context(|| format!("no clone {handle}"))
        .map(Ok)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;

    use cloneyard_api::account::Provider;
    use cloneyard_api::clone::{
        Clone, Events, EventsError, EventsParams, Handle, HandleParams, List, Mode, Run, RunError,
        RunParams, Show, ShowError, State, Stop, StopError,
    };
    use cloneyard_rpc::ClientError;
    use cloneyard_store::{NewClone, NewHookEvent};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::make_checkout;
    use crate::node::Node;
    use crate::router::tests::{connect, open_node};
    use crate::sandbox::Sandboxing;

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

    /// A git repo at `path` with one commit on `main`.
    fn repo(path: &Path) -> PathBuf {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet", "--initial-branch=main"]);
        fs::write(path.join("README.md"), "app\n").unwrap();
        git(path, &["add", "README.md"]);
        git(path, &["commit", "--quiet", "-m", "first"]);
        fs::canonicalize(path).unwrap()
    }

    async fn with_clone(node: &Node, state: State) -> Clone {
        let project = node
            .store
            .add_project(node.id, "app", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        let account = node
            .store
            .add_account(node.id, "personal", Provider::ClaudeCode, "/data/personal")
            .await
            .unwrap()
            .unwrap();
        let clone = node
            .store
            .add_clone(
                node.id,
                NewClone {
                    id: Ulid::generate(),
                    project_id: project.id,
                    repo_id: project.repos[0].id,
                    account_id: account.id,
                    mode: Mode::Headless,
                    checkout: "/data/clones/x/checkout".to_owned(),
                    session: "s1".to_owned(),
                    unsandboxed: false,
                },
                |handle| format!("branch-{handle}"),
            )
            .await
            .unwrap();
        node.store
            .set_clone_state(clone.id, state, None)
            .await
            .unwrap();
        Clone { state, ..clone }
    }

    fn params(project: &str, account: &str) -> RunParams {
        RunParams {
            project: project.to_owned(),
            account: account.to_owned(),
            prompt: "Say pong".to_owned(),
        }
    }

    #[tokio::test]
    async fn run_names_the_project_or_the_account_it_does_not_know() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        node.store
            .add_project(node.id, "app", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        let mut client = connect(node);

        let mut errors = Vec::new();
        for params in [params("web", "personal"), params("app", "personal")] {
            match client.request::<Run>(params).await {
                Err(ClientError::Method(error)) => errors.push(error),
                other => panic!("expected an error of clone.run, got {other:?}"),
            }
        }

        assert_eq!(
            errors,
            [
                RunError::UnknownProject {
                    name: "web".to_owned()
                },
                RunError::UnknownAccount {
                    name: "personal".to_owned()
                },
            ]
        );
    }

    #[tokio::test]
    async fn run_refuses_on_a_kernel_without_landlock_and_records_no_clone() {
        let dir = TempDir::new().unwrap();
        let node = Arc::new(
            Node::open(
                dir.path(),
                Sandboxing::Unavailable {
                    config_file: PathBuf::from("/home/me/.config/cloneyard/node.toml"),
                },
            )
            .await
            .unwrap(),
        );
        node.store
            .add_project(node.id, "app", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        node.store
            .add_account(node.id, "personal", Provider::ClaudeCode, "/data/personal")
            .await
            .unwrap()
            .unwrap();
        let mut client = connect(Arc::clone(&node));

        let result = client.request::<Run>(params("app", "personal")).await;

        assert!(
            matches!(
                &result,
                Err(ClientError::Method(RunError::NoSandbox { config }))
                    if config == "/home/me/.config/cloneyard/node.toml"
            ),
            "{result:?}"
        );
        assert_eq!(node.store.clones(node.id).await.unwrap(), []);
    }

    #[tokio::test]
    async fn show_and_list_give_the_clones_and_show_names_a_handle_it_does_not_know() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = with_clone(&node, State::Done).await;
        let mut client = connect(node);

        let shown = client
            .request::<Show>(HandleParams {
                handle: clone.handle,
            })
            .await
            .unwrap();
        let listed = client.request::<List>(()).await.unwrap().clones;
        let unknown = client
            .request::<Show>(HandleParams {
                handle: Handle::new(2),
            })
            .await;

        assert_eq!(shown, clone);
        assert_eq!(listed, [clone]);
        assert!(
            matches!(
                unknown,
                Err(ClientError::Method(ShowError::UnknownClone { handle })) if handle == Handle::new(2)
            ),
            "{unknown:?}"
        );
    }

    #[tokio::test]
    async fn stop_refuses_a_clone_that_is_not_working_and_names_a_handle_it_does_not_know() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = with_clone(&node, State::Done).await;
        let mut client = connect(node);

        let mut errors = Vec::new();
        for handle in [clone.handle, Handle::new(2)] {
            match client.request::<Stop>(HandleParams { handle }).await {
                Err(ClientError::Method(error)) => errors.push(error),
                other => panic!("expected an error of clone.stop, got {other:?}"),
            }
        }

        assert_eq!(
            errors,
            [
                StopError::NotRunning {
                    handle: clone.handle,
                    state: State::Done
                },
                StopError::UnknownClone {
                    handle: Handle::new(2)
                },
            ]
        );
    }

    #[tokio::test]
    async fn events_gives_a_clones_last_hook_events_and_names_a_handle_it_does_not_know() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = with_clone(&node, State::Done).await;
        for (file, hook) in [("1", "SessionStart"), ("2", "PreToolUse"), ("3", "Stop")] {
            node.store
                .add_hook_event(NewHookEvent {
                    clone_id: clone.id,
                    session: "s1".to_owned(),
                    hook: hook.to_owned(),
                    tool: None,
                    at: 1,
                    file: file.to_owned(),
                    payload: "{}".to_owned(),
                    state: None,
                })
                .await
                .unwrap()
                .unwrap();
        }
        let mut client = connect(node);

        let events = client
            .request::<Events>(EventsParams {
                handle: clone.handle,
                limit: 2,
            })
            .await
            .unwrap()
            .events;
        let unknown = client
            .request::<Events>(EventsParams {
                handle: Handle::new(2),
                limit: 2,
            })
            .await;

        let hooks: Vec<&str> = events.iter().map(|event| event.hook.as_str()).collect();
        assert_eq!(hooks, ["PreToolUse", "Stop"]);
        assert!(
            matches!(
                unknown,
                Err(ClientError::Method(EventsError::UnknownClone { handle })) if handle == Handle::new(2)
            ),
            "{unknown:?}"
        );
    }

    #[tokio::test]
    async fn stop_takes_a_blocked_clone_for_a_running_one() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let blocked = with_clone(&node, State::Blocked).await;
        let mut client = connect(node);

        let result = client
            .request::<Stop>(HandleParams {
                handle: blocked.handle,
            })
            .await;

        assert!(
            matches!(
                &result,
                Err(ClientError::Rpc(error))
                    if error.data == Some(serde_json::json!("clone CT-0001 is blocked, but this daemon runs no process for it"))
            ),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn the_node_marks_the_clones_that_were_working_when_it_closed_as_lost() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone = with_clone(&node, State::Working).await;
        drop(node);

        let node = open_node(&dir).await;

        let shown = node
            .store
            .clone_by_handle(node.id, clone.handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (shown.state, shown.reason.as_deref()),
            (State::Error, Some("lost while the daemon was down"))
        );
    }

    #[tokio::test]
    async fn the_checkout_is_a_shared_clone_on_a_branch_of_its_own_and_leaves_the_repo_alone() {
        let dir = TempDir::new().unwrap();
        let repo = repo(&dir.path().join("app"));
        let before = git(&repo, &["for-each-ref"]);
        let checkout = dir.path().join("data/clones/01K/checkout");

        make_checkout(&repo, &checkout, "cloneyard/ct-0001")
            .await
            .unwrap();
        git(
            &checkout,
            &["commit", "--quiet", "--allow-empty", "-m", "by the clone"],
        );

        let head = git(&checkout, &["rev-parse", "HEAD"]);
        let alternates = fs::read_to_string(checkout.join(".git/objects/info/alternates")).unwrap();
        let in_repo = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["cat-file", "-e", head.trim()])
            .status()
            .unwrap();
        assert_eq!(
            git(&checkout, &["branch", "--show-current"]),
            "cloneyard/ct-0001\n"
        );
        assert_eq!(
            alternates.trim(),
            repo.join(".git/objects").to_str().unwrap()
        );
        assert_eq!(git(&repo, &["for-each-ref"]), before);
        assert_eq!(git(&repo, &["status", "--porcelain"]), "");
        assert!(!in_repo.success());
        let mode = fs::metadata(checkout.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
    }

    #[tokio::test]
    async fn a_checkout_of_a_path_that_is_no_repo_fails_with_what_git_says() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("missing");
        let checkout = dir.path().join("clones/01K/checkout");

        let error = make_checkout(&missing, &checkout, "cloneyard/ct-0001")
            .await
            .unwrap_err();

        let message = format!("{error:#}");
        assert!(
            message.starts_with(&format!(
                "git clone --shared --quiet -- {} {} failed (exit status: 128): fatal: ",
                missing.display(),
                checkout.display()
            )),
            "{message}"
        );
    }
}
