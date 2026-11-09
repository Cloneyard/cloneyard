use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use anyhow::Context as _;
use rusqlite::Connection;
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
    use tempfile::TempDir;

    use super::Store;

    async fn open(dir: &TempDir) -> Store {
        Store::open(&dir.path().join("cloneyard.db")).await.unwrap()
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
