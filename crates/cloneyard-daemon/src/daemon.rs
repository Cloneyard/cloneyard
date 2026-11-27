use std::fs::{self, DirBuilder, File, Permissions, TryLockError};
use std::future::{Future, IntoFuture as _};
use std::io;
use std::net::Ipv4Addr;
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::net::{TcpListener, UnixListener};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::node::Node;
use crate::sandbox::Sandboxing;
use crate::{otlp, paths, process, router};

/// How long the receiver of telemetry has to answer its last requests at shutdown.
const RECEIVER_GRACE: Duration = Duration::from_secs(1);

/// A daemon that holds the single-instance locks, has the node's database open and listens on its socket and for its clones' telemetry.
pub struct Daemon {
    receiver: TcpListener,
    listener: UnixListener,
    socket: PathBuf,
    node: Node,
    data_lock: File,
    runtime_lock: File,
}

impl Daemon {
    /// Takes the locks in `runtime_dir` and `data_dir`, listens for telemetry on `telemetry_port` of `127.0.0.1`, opens the database and listens on the socket, to start clones as `sandboxing` says; call it inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// When another daemon holds a lock, or a directory, a lock, the telemetry port, the database or the socket can't be set up.
    pub async fn bind(
        runtime_dir: &Path,
        data_dir: &Path,
        sandboxing: Sandboxing,
        telemetry_port: u16,
    ) -> anyhow::Result<Self> {
        create_private_dir(runtime_dir)?;
        let socket = paths::socket(runtime_dir);
        let Some(runtime_lock) = try_lock(&paths::lock(runtime_dir))? else {
            bail!("cloneyard daemon is already running ({})", socket.display());
        };

        create_private_dir(data_dir)?;
        let Some(data_lock) = try_lock(&paths::data_lock(data_dir))? else {
            bail!("another cloneyard daemon is using {}", data_dir.display());
        };
        let receiver = TcpListener::bind((Ipv4Addr::LOCALHOST, telemetry_port))
            .await
            .with_context(|| {
                format!(
                    "cannot listen on 127.0.0.1:{telemetry_port}, the port of [telemetry] in node.toml"
                )
            })?;
        let telemetry_port = receiver
            .local_addr()
            .context("cannot read the telemetry port")?
            .port();
        let node = Node::open(data_dir, sandboxing, telemetry_port).await?;

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
            receiver,
            listener,
            socket,
            node,
            data_lock,
            runtime_lock,
        })
    }

    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The port on `127.0.0.1` where the daemon receives its clones' telemetry.
    #[must_use]
    pub fn telemetry_port(&self) -> u16 {
        self.node.telemetry_port
    }

    /// Serves connections until `shutdown` completes, then closes them and removes the socket.
    ///
    /// # Errors
    ///
    /// When the socket can't be removed.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
        let Self {
            receiver,
            listener,
            socket,
            node,
            data_lock,
            runtime_lock,
        } = self;
        let node_id = node.id;
        let telemetry_port = node.telemetry_port;
        let node = Arc::new(node);
        let router = Arc::new(router::router(Arc::clone(&node)));
        let (stop_receiving, receiving_stopped) = oneshot::channel::<()>();
        let mut receiving = tokio::spawn(
            axum::serve(receiver, otlp::router(Arc::clone(&node)))
                .with_graceful_shutdown(async {
                    let _ = receiving_stopped.await;
                })
                .into_future(),
        );
        let (stop_following, stopped) = oneshot::channel::<()>();
        let following = tokio::spawn({
            let node = Arc::clone(&node);
            async move {
                let stopped = async {
                    let _ = stopped.await;
                };
                node.spools.follow(&node.store, stopped).await;
            }
        });
        let mut connections = JoinSet::new();
        let mut shutdown = pin!(shutdown);
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            %node_id,
            socket = %socket.display(),
            telemetry = format!("127.0.0.1:{telemetry_port}"),
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
        node.processes.shutdown(process::GRACE).await;
        // Only now, as a clone sends its last telemetry while it ends.
        let _ = stop_receiving.send(());
        match tokio::time::timeout(RECEIVER_GRACE, &mut receiving).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => tracing::error!(%error, "the receiver of telemetry failed"),
            Ok(Err(error)) => tracing::error!(%error, "the receiver of telemetry failed"),
            Err(_) => receiving.abort(),
        }
        let _ = stop_following.send(());
        if let Err(error) = following.await {
            tracing::error!(%error, "the spool task failed");
        }
        drop(router);
        drop(listener);
        let removed =
            fs::remove_file(&socket).with_context(|| format!("cannot remove {}", socket.display()));
        // Locks go last, so a new daemon never takes over a socket or database that is still in use.
        drop(node);
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
