use std::fs::{self, DirBuilder, File, Permissions, TryLockError};
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::net::UnixListener;
use tokio::task::JoinSet;

use crate::{paths, system};

/// A daemon that holds the single-instance lock and listens on its socket.
pub struct Daemon {
    listener: UnixListener,
    socket: PathBuf,
    lock: File,
}

impl Daemon {
    /// Takes the lock in `runtime_dir` and listens on the socket there; call it inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// When another daemon holds the lock, or the directory, lock or socket can't be set up.
    pub fn bind(runtime_dir: &Path) -> anyhow::Result<Self> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(runtime_dir)
            .with_context(|| format!("cannot create {}", runtime_dir.display()))?;
        fs::set_permissions(runtime_dir, Permissions::from_mode(0o700))
            .with_context(|| format!("cannot set {} to mode 0700", runtime_dir.display()))?;

        let socket = paths::socket(runtime_dir);
        let lock_path = paths::lock(runtime_dir);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("cannot open {}", lock_path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!("cloneyard daemon is already running ({})", socket.display())
            }
            Err(TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("cannot lock {}", lock_path.display()));
            }
        }

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
            lock,
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
            lock,
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
        fs::remove_file(&socket).with_context(|| format!("cannot remove {}", socket.display()))?;
        // Released only now, so a new daemon never removes a socket this one still listens on.
        drop(lock);
        tracing::info!("daemon stopped");
        Ok(())
    }
}
