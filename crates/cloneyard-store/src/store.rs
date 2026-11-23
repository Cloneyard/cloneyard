use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::Context as _;
use cloneyard_api::account::{self, Account, Provider};
use cloneyard_api::project::{AddError, Project, Repo};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use ulid::Ulid;

use crate::schema;
use crate::write::{self, Changes, Clock, Entity};

struct Database {
    connection: Connection,
    clock: Clock,
}

type Job = Box<dyn FnOnce(&mut Database) + Send>;

const STOPPED: &str = "the database thread has stopped";

/// A SQLite database on a thread of its own: the node's, or a Restricted project's.
pub struct Store {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Store {
    /// Opens the database at `path`, creating it if it is missing, and migrates it to the latest schema.
    ///
    /// # Errors
    ///
    /// When the file can't be opened or migrated.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        Self::open_with(path, Box::new(write::system_clock)).await
    }

    async fn open_with(path: &Path, clock: Clock) -> anyhow::Result<Self> {
        let path = path.to_owned();
        let (jobs, queue) = mpsc::channel::<Job>();
        let (opened, ready) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("cloneyard-store".to_owned())
            .spawn(move || {
                let connection = match schema::open(&path, schema::migrations()) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = opened.send(Err(error));
                        return;
                    }
                };
                let _ = opened.send(Ok(()));
                let mut database = Database { connection, clock };
                for job in queue {
                    job(&mut database);
                }
            })
            .context("cannot start the database thread")?;
        ready.await.context(STOPPED)??;
        Ok(Self {
            jobs: Some(jobs),
            thread: Some(thread),
        })
    }

    /// The node's id, created the first time it is asked for.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written, or holds an invalid id.
    pub async fn node_id(&self) -> anyhow::Result<Ulid> {
        self.call(|connection| write::node_id(connection)).await
    }

    /// Adds project `name` with one repo, `repo`, at `path` on node `node_id`, or writes nothing when the path or the name is taken.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn add_project(
        &self,
        node_id: Ulid,
        name: &str,
        repo: &str,
        path: &str,
    ) -> anyhow::Result<Result<Project, AddError>> {
        let project = Project {
            id: Ulid::generate(),
            name: name.to_owned(),
            repos: vec![Repo {
                id: Ulid::generate(),
                name: repo.to_owned(),
                path: path.to_owned(),
            }],
        };
        self.write(move |changes| {
            let repo = &project.repos[0];
            let owner: Option<String> = changes
                .query_row(
                    "SELECT project.name FROM repo_path \
                     JOIN repo ON repo.id = repo_path.repo_id \
                     JOIN project ON project.id = repo.project_id \
                     WHERE repo_path.node_id = ?1 AND repo_path.path = ?2",
                    (node_id.to_string(), &repo.path),
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(owner) = owner {
                return Ok(Err(AddError::RepoTaken {
                    path: repo.path.clone(),
                    project: owner,
                }));
            }
            let taken: bool = changes.query_row(
                "SELECT EXISTS (SELECT 1 FROM project WHERE name = ?1)",
                [&project.name],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Err(AddError::NameTaken {
                    name: project.name.clone(),
                }));
            }
            changes.insert(
                Entity::Project,
                &[("id", json!(project.id)), ("name", json!(project.name))],
            )?;
            changes.insert(
                Entity::Repo,
                &[
                    ("id", json!(repo.id)),
                    ("project_id", json!(project.id)),
                    ("name", json!(repo.name)),
                ],
            )?;
            changes.insert(
                Entity::RepoPath,
                &[
                    ("repo_id", json!(repo.id)),
                    ("node_id", json!(node_id)),
                    ("path", json!(repo.path)),
                ],
            )?;
            Ok(Ok(project))
        })
        .await
        .with_context(|| format!("cannot add project {name}"))
    }

    /// The projects sorted by name, each with its repos that have a path on node `node_id`.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds an invalid id.
    pub async fn projects(&self, node_id: Ulid) -> anyhow::Result<Vec<Project>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT project.id, project.name, repo.id, repo.name, repo_path.path FROM project \
                 LEFT JOIN (repo JOIN repo_path ON repo_path.repo_id = repo.id AND repo_path.node_id = ?1) \
                 ON repo.project_id = project.id \
                 ORDER BY project.name, repo.name",
            )?;
            let rows = statement.query_map([node_id.to_string()], |row| {
                let repo: Option<(String, String, String)> = match row.get(2)? {
                    Some(id) => Some((id, row.get(3)?, row.get(4)?)),
                    None => None,
                };
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, repo))
            })?;
            let mut projects: Vec<Project> = Vec::new();
            for row in rows {
                let (id, name, repo) = row?;
                let id = ulid(&id)?;
                if projects.last().is_none_or(|project| project.id != id) {
                    projects.push(Project {
                        id,
                        name,
                        repos: Vec::new(),
                    });
                }
                if let (Some(project), Some((id, name, path))) = (projects.last_mut(), repo) {
                    project.repos.push(Repo {
                        id: ulid(&id)?,
                        name,
                        path,
                    });
                }
            }
            Ok(projects)
        })
        .await
        .context("cannot list the projects")
    }

    /// Adds account `name` of `provider` with its directory `dir` on node `node_id`, or writes nothing when the name or the directory is taken.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn add_account(
        &self,
        node_id: Ulid,
        name: &str,
        provider: Provider,
        dir: &str,
    ) -> anyhow::Result<Result<Account, account::AddError>> {
        let dir = dir.to_owned();
        let account = Account {
            id: Ulid::generate(),
            name: name.to_owned(),
            provider,
            dir: Some(dir.clone()),
        };
        self.write(move |changes| {
            let taken: bool = changes.query_row(
                "SELECT EXISTS (SELECT 1 FROM account WHERE name = ?1)",
                [&account.name],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Err(account::AddError::NameTaken {
                    name: account.name.clone(),
                }));
            }
            let owner: Option<String> = changes
                .query_row(
                    "SELECT account.name FROM account_dir \
                     JOIN account ON account.id = account_dir.account_id \
                     WHERE account_dir.node_id = ?1 AND account_dir.dir = ?2",
                    (node_id.to_string(), &dir),
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(owner) = owner {
                return Ok(Err(account::AddError::DirTaken {
                    dir,
                    account: owner,
                }));
            }
            changes.insert(
                Entity::Account,
                &[
                    ("id", json!(account.id)),
                    ("name", json!(account.name)),
                    ("provider", json!(account.provider)),
                ],
            )?;
            changes.insert(
                Entity::AccountDir,
                &[
                    ("account_id", json!(account.id)),
                    ("node_id", json!(node_id)),
                    ("dir", json!(dir)),
                ],
            )?;
            Ok(Ok(account))
        })
        .await
        .with_context(|| format!("cannot add account {name}"))
    }

    /// The accounts sorted by name, each with its directory on node `node_id` when it has one there.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds an invalid id or provider.
    pub async fn accounts(&self, node_id: Ulid) -> anyhow::Result<Vec<Account>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT account.id, account.name, account.provider, account_dir.dir FROM account \
                 LEFT JOIN account_dir \
                 ON account_dir.account_id = account.id AND account_dir.node_id = ?1 \
                 ORDER BY account.name",
            )?;
            let rows = statement.query_map([node_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            rows.map(|row| {
                let (id, name, provider, dir) = row?;
                Ok(Account {
                    id: ulid(&id)?,
                    name,
                    provider: provider_of(&provider)?,
                    dir,
                })
            })
            .collect()
        })
        .await
        .context("cannot list the accounts")
    }

    async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        self.run(move |database| work(&mut database.connection))
            .await
    }

    /// Runs `work` in one transaction, in which each change of an entity adds its `change_log` row.
    async fn write<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Changes<'_>) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        self.run(move |database| write::write(&mut database.connection, &database.clock, work))
            .await
    }

    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Database) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |database| {
            let _ = reply.send(work(database));
        });
        self.jobs
            .as_ref()
            .and_then(|jobs| jobs.send(job).ok())
            .context(STOPPED)?;
        answer.await.context(STOPPED)?
    }
}

fn ulid(id: &str) -> anyhow::Result<Ulid> {
    Ulid::from_string(id).with_context(|| format!("invalid id {id:?}"))
}

fn provider_of(name: &str) -> anyhow::Result<Provider> {
    serde_json::from_value(Value::String(name.to_owned()))
        .with_context(|| format!("invalid provider {name:?}"))
}

impl Drop for Store {
    // Waits for the thread, so the connection is closed once the store is gone.
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use cloneyard_api::account::{self, Account, Provider};
    use cloneyard_api::project::{AddError, Project, Repo};
    use rusqlite::Connection;
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::Store;
    use crate::write::Entity;

    const NOW: i64 = 1_790_000_000_000;

    #[derive(Debug, PartialEq)]
    struct Logged {
        node_id: String,
        hlc: i64,
        entity: String,
        entity_id: String,
        op: String,
        fields: Value,
    }

    async fn open(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("cloneyard.db")).await.unwrap()
    }

    async fn open_at(dir: &TempDir, now: &Arc<AtomicI64>) -> Store {
        let now = Arc::clone(now);
        let clock = Box::new(move || now.load(Ordering::SeqCst));
        Store::open_with(&dir.path().join("cloneyard.db"), clock)
            .await
            .unwrap()
    }

    fn sql(dir: &TempDir) -> Connection {
        Connection::open(dir.path().join("cloneyard.db")).unwrap()
    }

    fn change_log(dir: &TempDir) -> Vec<Logged> {
        sql(dir)
            .prepare(
                "SELECT node_id, hlc, entity, entity_id, op, fields FROM change_log ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| {
                Ok(Logged {
                    node_id: row.get(0)?,
                    hlc: row.get(1)?,
                    entity: row.get(2)?,
                    entity_id: row.get(3)?,
                    op: row.get(4)?,
                    fields: serde_json::from_str(&row.get::<_, String>(5)?).unwrap(),
                })
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn hlcs(dir: &TempDir) -> Vec<i64> {
        change_log(dir).into_iter().map(|row| row.hlc).collect()
    }

    async fn add(
        store: &Store,
        node_id: Ulid,
        name: &str,
        path: &str,
    ) -> Result<Project, AddError> {
        let repo = path.rsplit('/').next().unwrap();
        store.add_project(node_id, name, repo, path).await.unwrap()
    }

    #[tokio::test]
    async fn adds_a_project_with_its_repo_and_its_path_and_keeps_them_after_reopening() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();

        let project = store
            .add_project(node_id, "web", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        drop(store);
        let listed = open(&dir).await.projects(node_id).await.unwrap();

        let [repo] = project.repos.as_slice() else {
            panic!("{project:?}");
        };
        assert_eq!(project.name, "web");
        assert_eq!(
            (repo.name.as_str(), repo.path.as_str()),
            ("app", "/code/app")
        );
        assert_ne!(repo.id, project.id);
        assert_eq!(listed, [project]);
    }

    #[tokio::test]
    async fn lists_the_projects_by_name() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        add(&store, node_id, "web", "/code/web").await.unwrap();
        add(&store, node_id, "api", "/code/api").await.unwrap();

        let names: Vec<String> = store
            .projects(node_id)
            .await
            .unwrap()
            .into_iter()
            .map(|project| project.name)
            .collect();

        assert_eq!(names, ["api", "web"]);
    }

    #[tokio::test]
    async fn lists_only_the_repo_paths_of_the_node_asked_about() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let project = add(&store, Ulid::generate(), "app", "/code/app")
            .await
            .unwrap();

        let listed = store.projects(Ulid::generate()).await.unwrap();

        assert_eq!(
            listed,
            [Project {
                repos: Vec::<Repo>::new(),
                ..project
            }]
        );
    }

    #[tokio::test]
    async fn refuses_a_taken_name_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        let first = add(&store, node_id, "app", "/code/app").await.unwrap();

        let error = add(&store, node_id, "app", "/work/app").await.unwrap_err();

        assert_eq!(
            error,
            AddError::NameTaken {
                name: "app".to_owned()
            }
        );
        assert_eq!(store.projects(node_id).await.unwrap(), [first]);
        assert_eq!(change_log(&dir).len(), 3);
    }

    #[tokio::test]
    async fn refuses_a_path_that_is_already_in_a_project_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        let first = add(&store, node_id, "app", "/code/app").await.unwrap();

        let error = add(&store, node_id, "web", "/code/app").await.unwrap_err();

        assert_eq!(
            error,
            AddError::RepoTaken {
                path: "/code/app".to_owned(),
                project: "app".to_owned()
            }
        );
        assert_eq!(store.projects(node_id).await.unwrap(), [first]);
        assert_eq!(change_log(&dir).len(), 3);
    }

    #[tokio::test]
    async fn logs_each_entity_of_a_new_project_as_an_upsert_by_this_node() {
        let dir = TempDir::new().unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let store = open_at(&dir, &now).await;
        let node_id = store.node_id().await.unwrap();

        let project = add(&store, node_id, "web", "/code/app").await.unwrap();

        let node = node_id.to_string();
        let project_id = project.id.to_string();
        let repo_id = project.repos[0].id.to_string();
        let logged = |counter, entity: &str, entity_id: &str, fields| Logged {
            node_id: node.clone(),
            hlc: (NOW << 16) + counter,
            entity: entity.to_owned(),
            entity_id: entity_id.to_owned(),
            op: "upsert".to_owned(),
            fields,
        };
        assert_eq!(
            change_log(&dir),
            [
                logged(
                    0,
                    "project",
                    &project_id,
                    json!({"id": project_id, "name": "web"})
                ),
                logged(
                    1,
                    "repo",
                    &repo_id,
                    json!({"id": repo_id, "project_id": project_id, "name": "app"})
                ),
                logged(
                    2,
                    "repo_path",
                    &format!("{repo_id}:{node}"),
                    json!({"repo_id": repo_id, "node_id": node, "path": "/code/app"})
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_failed_write_leaves_neither_its_changes_nor_their_change_log_rows() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();

        let error = store
            .write(|changes| {
                changes.insert(
                    Entity::Project,
                    &[("id", json!(Ulid::generate())), ("name", json!("web"))],
                )?;
                changes.insert(
                    Entity::Repo,
                    &[
                        ("id", json!(Ulid::generate())),
                        ("project_id", json!(Ulid::generate())),
                        ("name", json!("app")),
                    ],
                )
            })
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "FOREIGN KEY constraint failed");
        assert_eq!(store.projects(node_id).await.unwrap(), []);
        assert_eq!(change_log(&dir), []);
    }

    #[tokio::test]
    async fn the_hlc_keeps_growing_when_the_clock_goes_back_also_after_reopening() {
        let dir = TempDir::new().unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let store = open_at(&dir, &now).await;
        let node_id = store.node_id().await.unwrap();
        add(&store, node_id, "web", "/code/web").await.unwrap();

        now.store(NOW - 60_000, Ordering::SeqCst);
        add(&store, node_id, "api", "/code/api").await.unwrap();
        drop(store);
        let store = open_at(&dir, &now).await;
        add(&store, node_id, "docs", "/code/docs").await.unwrap();

        let expected: Vec<i64> = (0..9).map(|counter| (NOW << 16) + counter).collect();
        assert_eq!(hlcs(&dir), expected);
    }

    #[tokio::test]
    async fn the_hlc_follows_the_clock_once_the_clock_is_past_it() {
        let dir = TempDir::new().unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let store = open_at(&dir, &now).await;
        let node_id = store.node_id().await.unwrap();
        add(&store, node_id, "web", "/code/web").await.unwrap();

        now.store(NOW + 1, Ordering::SeqCst);
        add(&store, node_id, "api", "/code/api").await.unwrap();

        let ticks = [NOW << 16, (NOW + 1) << 16];
        let expected: Vec<i64> = ticks
            .iter()
            .flat_map(|tick| (0..3).map(move |counter| tick + counter))
            .collect();
        assert_eq!(hlcs(&dir), expected);
    }

    #[tokio::test]
    async fn names_the_hlc_when_the_database_holds_an_invalid_one_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        sql(&dir)
            .execute("INSERT INTO meta (key, value) VALUES ('hlc', 'soon')", [])
            .unwrap();

        let error = store
            .add_project(node_id, "web", "app", "/code/app")
            .await
            .unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "cannot add project web: invalid hlc \"soon\": invalid digit found in string"
        );
        assert_eq!(store.projects(node_id).await.unwrap(), []);
        assert_eq!(change_log(&dir), []);
    }

    async fn add_account(
        store: &Store,
        node_id: Ulid,
        name: &str,
        dir: &str,
    ) -> Result<Account, account::AddError> {
        store
            .add_account(node_id, name, Provider::ClaudeCode, dir)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn adds_an_account_with_its_dir_and_keeps_them_after_reopening() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();

        let account = add_account(&store, node_id, "personal", "/data/personal/claude")
            .await
            .unwrap();
        drop(store);
        let listed = open(&dir).await.accounts(node_id).await.unwrap();

        assert_eq!(account.name, "personal");
        assert_eq!(account.provider, Provider::ClaudeCode);
        assert_eq!(account.dir.as_deref(), Some("/data/personal/claude"));
        assert_eq!(listed, [account]);
    }

    #[tokio::test]
    async fn lists_the_accounts_by_name() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        add_account(&store, node_id, "work", "/data/work/claude")
            .await
            .unwrap();
        add_account(&store, node_id, "personal", "/data/personal/claude")
            .await
            .unwrap();

        let names: Vec<String> = store
            .accounts(node_id)
            .await
            .unwrap()
            .into_iter()
            .map(|account| account.name)
            .collect();

        assert_eq!(names, ["personal", "work"]);
    }

    #[tokio::test]
    async fn lists_an_account_without_a_dir_on_a_node_where_it_has_none() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let account = add_account(
            &store,
            Ulid::generate(),
            "personal",
            "/data/personal/claude",
        )
        .await
        .unwrap();

        let listed = store.accounts(Ulid::generate()).await.unwrap();

        assert_eq!(
            listed,
            [Account {
                dir: None,
                ..account
            }]
        );
    }

    #[tokio::test]
    async fn refuses_a_taken_account_name_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        let first = add_account(&store, node_id, "personal", "/data/personal/claude")
            .await
            .unwrap();

        let error = add_account(&store, node_id, "personal", "/data/other/claude")
            .await
            .unwrap_err();

        assert_eq!(
            error,
            account::AddError::NameTaken {
                name: "personal".to_owned()
            }
        );
        assert_eq!(store.accounts(node_id).await.unwrap(), [first]);
        assert_eq!(change_log(&dir).len(), 2);
    }

    #[tokio::test]
    async fn refuses_a_dir_that_another_account_has_and_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        let first = add_account(&store, node_id, "personal", "/data/personal/claude")
            .await
            .unwrap();

        let error = add_account(&store, node_id, "work", "/data/personal/claude")
            .await
            .unwrap_err();

        assert_eq!(
            error,
            account::AddError::DirTaken {
                dir: "/data/personal/claude".to_owned(),
                account: "personal".to_owned()
            }
        );
        assert_eq!(store.accounts(node_id).await.unwrap(), [first]);
        assert_eq!(change_log(&dir).len(), 2);
    }

    #[tokio::test]
    async fn logs_an_account_and_its_dir_as_upserts_by_this_node() {
        let dir = TempDir::new().unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let store = open_at(&dir, &now).await;
        let node_id = store.node_id().await.unwrap();

        let account = add_account(&store, node_id, "personal", "/data/personal/claude")
            .await
            .unwrap();

        let node = node_id.to_string();
        let account_id = account.id.to_string();
        let logged = |counter, entity: &str, entity_id: String, fields| Logged {
            node_id: node.clone(),
            hlc: (NOW << 16) + counter,
            entity: entity.to_owned(),
            entity_id,
            op: "upsert".to_owned(),
            fields,
        };
        assert_eq!(
            change_log(&dir),
            [
                logged(
                    0,
                    "account",
                    account_id.clone(),
                    json!({"id": account_id, "name": "personal", "provider": "claude-code"})
                ),
                logged(
                    1,
                    "account_dir",
                    format!("{account_id}:{node}"),
                    json!({"account_id": account_id, "node_id": node, "dir": "/data/personal/claude"})
                ),
            ]
        );
    }

    #[tokio::test]
    async fn names_the_provider_when_the_database_holds_an_unknown_one() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let node_id = store.node_id().await.unwrap();
        sql(&dir)
            .execute(
                "INSERT INTO account (id, name, provider) VALUES (?1, 'personal', 'codex')",
                [Ulid::generate().to_string()],
            )
            .unwrap();

        let error = store.accounts(node_id).await.unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "cannot list the accounts: invalid provider \"codex\": \
             unknown variant `codex`, expected `claude-code`"
        );
    }

    #[tokio::test]
    async fn creates_the_node_id_once_and_keeps_it_after_reopening() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let first = store.node_id().await.unwrap();
        let again = store.node_id().await.unwrap();
        drop(store);

        let reopened = open(&dir).await.node_id().await.unwrap();

        assert_eq!(again, first);
        assert_eq!(reopened, first);
    }
}
