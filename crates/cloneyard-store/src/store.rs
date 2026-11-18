use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::Context as _;
use cloneyard_api::project::{AddError, Project, Repo};
use rusqlite::{Connection, OptionalExtension as _};
use tokio::sync::oneshot;
use ulid::Ulid;

use crate::schema;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

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
        let path = path.to_owned();
        let (jobs, queue) = mpsc::channel::<Job>();
        let (opened, ready) = oneshot::channel();
        let thread = thread::Builder::new()
            .name("cloneyard-store".to_owned())
            .spawn(move || {
                let mut connection = match schema::open(&path, schema::migrations()) {
                    Ok(connection) => connection,
                    Err(error) => {
                        let _ = opened.send(Err(error));
                        return;
                    }
                };
                let _ = opened.send(Ok(()));
                for job in queue {
                    job(&mut connection);
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
        self.call(|connection| {
            connection.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES ('node_id', ?1)",
                [Ulid::generate().to_string()],
            )?;
            let id: String = connection.query_row(
                "SELECT value FROM meta WHERE key = 'node_id'",
                [],
                |row| row.get(0),
            )?;
            Ulid::from_string(&id).with_context(|| format!("invalid node id {id:?}"))
        })
        .await
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
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let repo = &project.repos[0];
            let owner: Option<String> = transaction
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
            let taken: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM project WHERE name = ?1)",
                [&project.name],
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Err(AddError::NameTaken {
                    name: project.name.clone(),
                }));
            }
            transaction.execute(
                "INSERT INTO project (id, name) VALUES (?1, ?2)",
                (project.id.to_string(), &project.name),
            )?;
            transaction.execute(
                "INSERT INTO repo (id, project_id, name) VALUES (?1, ?2, ?3)",
                (repo.id.to_string(), project.id.to_string(), &repo.name),
            )?;
            transaction.execute(
                "INSERT INTO repo_path (repo_id, node_id, path) VALUES (?1, ?2, ?3)",
                (repo.id.to_string(), node_id.to_string(), &repo.path),
            )?;
            transaction.commit()?;
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

    async fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (reply, answer) = oneshot::channel();
        let job: Job = Box::new(move |connection| {
            let _ = reply.send(work(connection));
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
    use cloneyard_api::project::{AddError, Project, Repo};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::Store;

    async fn open(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("cloneyard.db")).await.unwrap()
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
