use std::fs::{self, DirBuilder, File, Permissions, TryLockError};
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use cloneyard_store::Store;
use tokio::net::UnixListener;
use tokio::task::JoinSet;

use crate::{paths, system};

/// A daemon that holds the single-instance locks, has the node's database open and listens on its socket.
pub struct Daemon {
    listener: UnixListener,
    socket: PathBuf,
    store: Store,
    data_lock: File,
    runtime_lock: File,
}

impl Daemon {
    /// Takes the locks in `runtime_dir` and `data_dir`, opens the database and listens on the socket; call it inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// When another daemon holds a lock, or a directory, a lock, the database or the socket can't be set up.
    pub async fn bind(runtime_dir: &Path, data_dir: &Path) -> anyhow::Result<Self> {
        create_private_dir(runtime_dir)?;
        let socket = paths::socket(runtime_dir);
        let Some(runtime_lock) = try_lock(&paths::lock(runtime_dir))? else {
            bail!("cloneyard daemon is already running ({})", socket.display());
        };

        create_private_dir(data_dir)?;
        let Some(data_lock) = try_lock(&paths::data_lock(data_dir))? else {
            bail!("another cloneyard daemon is using {}", data_dir.display());
        };
        let store = Store::open(&paths::database(data_dir)).await?;

        // Only the lock holder gets here, so a socket left on disk is stale.
        match fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot remove {}", socket.display()));
            }
        }
        let listener = UnixListener::bind(&socket)
            .with_context(|| format!("cannot listen on {}", socket.display()))?;
        Ok(Self {
            listener,
            socket,
            store,
            data_lock,
            runtime_lock,
        })
    }

    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Serves connections until `shutdown` completes, then closes them and removes the socket.
    ///
    /// # Errors
    ///
    /// When the socket can't be removed.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
        let Self {
            listener,
            socket,
            store,
            data_lock,
            runtime_lock,
        } = self;
        let router = Arc::new(system::router());
        let mut connections = JoinSet::new();
        let mut shutdown = pin!(shutdown);
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            socket = %socket.display(),
            "daemon listening"
        );
        loop {
            tokio::select! {
                () = &mut shutdown => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        let router = Arc::clone(&router);
                        connections.spawn(async move {
                            tracing::debug!("connection opened");
                            let (reader, writer) = stream.into_split();
                            match cloneyard_rpc::serve(&router, reader, writer).await {
                                Ok(()) => tracing::debug!("connection closed"),
                                Err(error) => tracing::debug!(%error, "connection failed"),
                            }
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cannot accept a connection");
                        // Errors like EMFILE come back at once, so retrying without a pause spins.
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }

        connections.shutdown().await;
        drop(listener);
        let removed =
            fs::remove_file(&socket).with_context(|| format!("cannot remove {}", socket.display()));
        // Locks go last, so a new daemon never takes over a socket or database that is still in use.
        drop(store);
        drop(data_lock);
        drop(runtime_lock);
        removed?;
        tracing::info!("daemon stopped");
        Ok(())
    }
}

fn create_private_dir(path: &Path) -> anyhow::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    fs::set_permissions(path, Permissions::from_mode(0o700))
        .with_context(|| format!("cannot set {} to mode 0700", path.display()))
}

/// Locks `path`, or gives `None` when another process holds it.
fn try_lock(path: &Path) -> anyhow::Result<Option<File>> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => {
            Err(error).with_context(|| format!("cannot lock {}", path.display()))
        }
    }
}
