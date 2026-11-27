//! A clone's spool: the directory in the clone's own one that its hooks write their events into, a file each, which the daemon reads in name order.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, DirBuilder, OpenOptions};
use std::future::Future;
use std::io::{self, Write as _};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::process;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail};
use cloneyard_api::clone::State;
use cloneyard_provider_claude::hook::{self, Hook};
use cloneyard_store::{NewHookEvent, Store};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use rustix::io::Errno;
use tokio::io::unix::AsyncFd;
use ulid::Ulid;

/// The variable that tells `cy hook` the spool of its clone.
pub const VARIABLE: &str = "CLONEYARD_SPOOL";

/// Writes `payload`, the JSON that hook `event` got, into the spool at `dir`, as a file whose name holds the time, the pid and the event, so it sorts after the files of earlier events.
///
/// # Errors
///
/// When `event` isn't a word of letters and digits, or the file can't be written.
pub fn write(dir: &Path, event: &str, payload: &[u8]) -> anyhow::Result<PathBuf> {
    if event.is_empty() || !event.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        bail!("invalid hook name {event:?}");
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let name = format!("{nanos:020}-{}-{event}", process::id());
    // The daemon takes only `*.json`, so it never reads a file that is still being written.
    let temporary = dir.join(format!(".{name}.tmp"));
    let path = dir.join(format!("{name}.json"));
    let written = write_new(&temporary, payload).and_then(|()| {
        fs::rename(&temporary, &path).with_context(|| {
            format!(
                "cannot rename {} to {}",
                temporary.display(),
                path.display()
            )
        })
    });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map(|()| path)
}

fn write_new(path: &Path, payload: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(payload)
        .with_context(|| format!("cannot write {}", path.display()))
}

/// The largest file a pass reads, so a clone can't make the daemon read a huge one.
const LIMIT: u64 = 16 * 1024 * 1024;

/// The spool in a clone's directory.
pub(crate) fn dir(clone_dir: &Path) -> PathBuf {
    clone_dir.join("spool")
}

/// The spools of the running clones, which inotify reports new files in.
pub(crate) struct Spools {
    inotify: AsyncFd<OwnedFd>,
    watched: Mutex<HashMap<i32, (Ulid, PathBuf)>>,
    /// Held during a pass, so no two passes take the same file.
    passing: tokio::sync::Mutex<()>,
}

impl Spools {
    /// Spools with no watches yet; call it inside a tokio runtime.
    #[expect(
        deprecated,
        reason = "AsyncFd::new is sound for an OwnedFd, which keeps its descriptor open until it drops, and its replacement is unsafe"
    )]
    pub(crate) fn new() -> anyhow::Result<Self> {
        let inotify = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .context("cannot start inotify")?;
        Ok(Self {
            inotify: AsyncFd::new(inotify).context("cannot poll inotify")?,
            watched: Mutex::default(),
            passing: tokio::sync::Mutex::new(()),
        })
    }

    /// Creates the spool of clone `clone` in its directory `clone_dir`, mode 0700, and watches it.
    pub(crate) fn watch(&self, clone: Ulid, clone_dir: &Path) -> anyhow::Result<PathBuf> {
        let spool = dir(clone_dir);
        DirBuilder::new()
            .mode(0o700)
            .create(&spool)
            .with_context(|| format!("cannot create {}", spool.display()))?;
        let watch = inotify::add_watch(self.inotify.get_ref(), &spool, WatchFlags::MOVED_TO)
            .with_context(|| format!("cannot watch {}", spool.display()))?;
        self.lock().insert(watch, (clone, spool.clone()));
        Ok(spool)
    }

    /// Stops watching the spool of clone `clone` after a last pass over it.
    pub(crate) async fn unwatch(&self, store: &Store, clone: Ulid) {
        let found = {
            let mut watched = self.lock();
            let watch = watched
                .iter()
                .find(|(_, (watched, _))| *watched == clone)
                .map(|(watch, _)| *watch);
            watch.and_then(|watch| watched.remove(&watch).map(|(_, spool)| (watch, spool)))
        };
        let Some((watch, spool)) = found else {
            return;
        };
        if let Err(error) = inotify::remove_watch(self.inotify.get_ref(), watch) {
            tracing::debug!(%clone, %error, "cannot remove the watch of a spool");
        }
        let _passing = self.passing.lock().await;
        pass(store, clone, &spool).await;
    }

    /// Takes the events of a watched spool each time inotify reports a new file in it, until `stopped` completes.
    pub(crate) async fn follow(&self, store: &Store, stopped: impl Future<Output = ()>) {
        let mut stopped = pin!(stopped);
        let mut buffer = [MaybeUninit::uninit(); 4096];
        loop {
            let mut ready = tokio::select! {
                () = &mut stopped => return,
                ready = self.inotify.readable() => match ready {
                    Ok(ready) => ready,
                    Err(error) => {
                        tracing::error!(%error, "cannot poll inotify, so hook events wait for the next start");
                        return;
                    }
                },
            };
            let mut changed: Vec<(Ulid, PathBuf)> = Vec::new();
            let mut reader = inotify::Reader::new(self.inotify.get_ref(), &mut buffer);
            loop {
                match reader.next() {
                    Ok(event) if event.events().contains(ReadFlags::QUEUE_OVERFLOW) => {
                        changed.extend(self.lock().values().cloned());
                    }
                    Ok(event) => {
                        changed.extend(self.lock().get(&event.wd()).cloned());
                    }
                    Err(Errno::WOULDBLOCK) => {
                        ready.clear_ready();
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cannot read inotify");
                        ready.clear_ready();
                        break;
                    }
                }
            }
            changed.sort_unstable();
            changed.dedup();
            let _passing = self.passing.lock().await;
            for (clone, spool) in changed {
                pass(store, clone, &spool).await;
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<i32, (Ulid, PathBuf)>> {
        self.watched.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Takes the events that the spools of all clones in `data_dir` hold, as the daemon does at start.
pub(crate) async fn pass_all(store: &Store, data_dir: &Path) {
    let clones = data_dir.join("clones");
    let entries = match fs::read_dir(&clones) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(dir = %clones.display(), %error, "cannot list the clones");
            return;
        }
    };
    let mut spools: Vec<(Ulid, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let clone = Ulid::from_string(entry.file_name().to_str()?).ok()?;
            Some((clone, dir(&entry.path())))
        })
        .collect();
    spools.sort_unstable();
    for (clone, spool) in spools {
        pass(store, clone, &spool).await;
    }
}

/// Takes the events in clone `clone`'s `spool` in name order, deleting each file after its commit; a file the database can't take stays, with the ones after it.
async fn pass(store: &Store, clone: Ulid, spool: &Path) {
    let files = match pending(spool) {
        Ok(files) => files,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(error) => {
            tracing::warn!(%clone, spool = %spool.display(), %error, "cannot list a spool");
            return;
        }
    };
    for (name, path) in files {
        match take(store, clone, &name, &path).await {
            Ok(None) => {}
            Ok(Some(reason)) => {
                tracing::warn!(%clone, file = %path.display(), reason, "dropped a hook event");
            }
            Err(error) => {
                tracing::error!(%clone, file = %path.display(), error = format!("{error:#}"), "cannot record a hook event");
                return;
            }
        }
        if let Err(error) = fs::remove_file(&path) {
            tracing::warn!(%clone, file = %path.display(), %error, "cannot delete a hook event");
        }
    }
}

/// The `*.json` files in `spool`, by name.
fn pending(spool: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(spool)? {
        let entry = entry?;
        if let Some(name) = entry.file_name().to_str()
            && Path::new(name).extension() == Some(OsStr::new("json"))
            && !name.starts_with('.')
        {
            files.push((name.to_owned(), entry.path()));
        }
    }
    files.sort_unstable();
    Ok(files)
}

/// Records the event in the file `name` at `path`, or gives why it drops the file.
async fn take(
    store: &Store,
    clone: Ulid,
    name: &str,
    path: &Path,
) -> anyhow::Result<Option<String>> {
    let Some((at, event)) = parse_name(name) else {
        return Ok(Some("invalid name".to_owned()));
    };
    let payload = match fs::metadata(path) {
        Ok(metadata) if metadata.len() > LIMIT => {
            return Ok(Some(format!(
                "{} bytes, more than the {LIMIT} a hook event may have",
                metadata.len()
            )));
        }
        Ok(_) => fs::read(path),
        Err(error) => Err(error),
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(error) => return Ok(Some(format!("cannot read it: {error}"))),
    };
    let Hook {
        session_id,
        tool,
        state,
        payload,
    } = match hook::parse(event, &payload) {
        Ok(hook) => hook,
        Err(invalid) => return Ok(Some(invalid.to_string())),
    };
    let added = store
        .add_hook_event(NewHookEvent {
            clone_id: clone,
            session: session_id,
            hook: event.to_owned(),
            tool,
            at,
            file: name.to_owned(),
            payload,
            state: state.map(|state| match state {
                hook::State::Working => State::Working,
                hook::State::Blocked => State::Blocked,
                hook::State::Idle => State::Idle,
            }),
        })
        .await?;
    Ok(added.err().map(|refused| refused.to_string()))
}

/// The time in milliseconds and the event in a name that `write` gave.
fn parse_name(name: &str) -> Option<(i64, &str)> {
    let mut parts = name.strip_suffix(".json")?.splitn(3, '-');
    let (nanos, pid, event) = (parts.next()?, parts.next()?, parts.next()?);
    if nanos.len() != 20 || pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let millis = nanos.parse::<u128>().ok()? / 1_000_000;
    Some((i64::try_from(millis).ok()?, event))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use cloneyard_api::account::Provider;
    use cloneyard_api::clone::{Clone, Mode, State};
    use cloneyard_store::NewClone;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio::sync::oneshot;
    use ulid::Ulid;

    use super::{LIMIT, dir, pass, write};
    use crate::node::Node;
    use crate::router::tests::open_node;

    /// A working clone whose directory is `clones/<id>` in the node's data dir, on session `session`.
    pub(crate) async fn add_clone(node: &Node, data_dir: &Path, session: &str) -> (Clone, PathBuf) {
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
        let id = Ulid::generate();
        let clone_dir = data_dir.join("clones").join(id.to_string());
        fs::create_dir_all(&clone_dir).unwrap();
        let clone = node
            .store
            .add_clone(
                node.id,
                NewClone {
                    id,
                    project_id: project.id,
                    repo_id: project.repos[0].id,
                    account_id: account.id,
                    mode: Mode::Headless,
                    checkout: clone_dir.join("checkout").to_str().unwrap().to_owned(),
                    session: session.to_owned(),
                    unsandboxed: false,
                },
                |handle| format!("branch-{handle}"),
            )
            .await
            .unwrap();
        (clone, clone_dir)
    }

    fn spool(clone_dir: &Path) -> PathBuf {
        let spool = dir(clone_dir);
        fs::create_dir_all(&spool).unwrap();
        spool
    }

    fn payload(session: &str) -> Vec<u8> {
        json!({"session_id": session, "tool_name": "Bash"})
            .to_string()
            .into_bytes()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    async fn hooks(node: &Node, clone: &Clone) -> Vec<String> {
        node.store
            .hook_events(clone.id, 100)
            .await
            .unwrap()
            .into_iter()
            .map(|event| event.hook)
            .collect()
    }

    #[tokio::test]
    async fn a_pass_takes_the_files_in_name_order_and_deletes_each() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, clone_dir) = add_clone(&node, dir.path(), "s1").await;
        let spool = spool(&clone_dir);
        for event in ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"] {
            write(&spool, event, &payload("s1")).unwrap();
        }

        pass(&node.store, clone.id, &spool).await;

        let events = node.store.hook_events(clone.id, 100).await.unwrap();
        let taken: Vec<(&str, Option<&str>)> = events
            .iter()
            .map(|event| (event.hook.as_str(), event.tool.as_deref()))
            .collect();
        assert_eq!(
            taken,
            [
                ("UserPromptSubmit", Some("Bash")),
                ("PreToolUse", Some("Bash")),
                ("PostToolUse", Some("Bash")),
                ("Stop", Some("Bash")),
            ]
        );
        assert!(events.windows(2).all(|pair| pair[0].at <= pair[1].at));
        assert_eq!(names(&spool), Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_pass_drops_a_broken_an_oversized_and_a_foreign_file_and_leaves_a_temp_file() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, clone_dir) = add_clone(&node, dir.path(), "s1").await;
        add_clone(&node, dir.path(), "other").await;
        let spool = spool(&clone_dir);
        fs::write(
            spool.join(".00000000000000000001-1-Stop.tmp"),
            payload("s1"),
        )
        .unwrap();
        fs::write(spool.join("stop.json"), payload("s1")).unwrap();
        fs::write(spool.join("00000000000000000002-1-Stop.json"), "not json").unwrap();
        File::create(spool.join("00000000000000000003-1-Stop.json"))
            .unwrap()
            .set_len(LIMIT + 1)
            .unwrap();
        fs::write(
            spool.join("00000000000000000004-1-Stop.json"),
            payload("other"),
        )
        .unwrap();
        fs::write(
            spool.join("00000000000000000005-1-Wait.json"),
            payload("s1"),
        )
        .unwrap();
        fs::write(
            spool.join("00000000000000000006-1-Stop.json"),
            payload("s1"),
        )
        .unwrap();

        pass(&node.store, clone.id, &spool).await;

        assert_eq!(hooks(&node, &clone).await, ["Stop"]);
        assert_eq!(names(&spool), [".00000000000000000001-1-Stop.tmp"]);
    }

    #[tokio::test]
    async fn a_file_written_into_a_watched_spool_is_taken_while_the_node_follows_it() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, clone_dir) = add_clone(&node, dir.path(), "s1").await;
        let spool = node.spools.watch(clone.id, &clone_dir).unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let following = tokio::spawn({
            let node = std::sync::Arc::clone(&node);
            async move {
                let stopped = async {
                    let _ = stopped.await;
                };
                node.spools.follow(&node.store, stopped).await;
            }
        });

        write(&spool, "PreToolUse", &payload("s1")).unwrap();

        for _ in 0..1000 {
            if !hooks(&node, &clone).await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        stop.send(()).unwrap();
        following.await.unwrap();
        assert_eq!(hooks(&node, &clone).await, ["PreToolUse"]);
        assert_eq!(names(&spool), Vec::<String>::new());
    }

    #[tokio::test]
    async fn unwatch_takes_what_the_spool_still_holds() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, clone_dir) = add_clone(&node, dir.path(), "s1").await;
        let spool = node.spools.watch(clone.id, &clone_dir).unwrap();
        write(&spool, "SessionEnd", &payload("s1")).unwrap();

        node.spools.unwatch(&node.store, clone.id).await;

        assert_eq!(hooks(&node, &clone).await, ["SessionEnd"]);
        assert_eq!(names(&spool), Vec::<String>::new());
    }

    #[tokio::test]
    async fn opening_the_node_marks_its_clones_lost_and_then_takes_what_their_spools_hold() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let (clone, clone_dir) = add_clone(&node, dir.path(), "s1").await;
        let spool = spool(&clone_dir);
        for event in ["PreToolUse", "Stop"] {
            write(&spool, event, &payload("s1")).unwrap();
        }
        drop(node);

        let node = open_node(&dir).await;

        let shown = node
            .store
            .clone_by_handle(node.id, clone.handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(hooks(&node, &clone).await, ["PreToolUse", "Stop"]);
        assert_eq!(shown.state, State::Error);
        assert_eq!(names(&spool), Vec::<String>::new());
    }
}
