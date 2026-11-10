use std::io;
use std::time::Duration;

use anyhow::{Context as _, bail};
use cloneyard_daemon::paths;
use cloneyard_rpc::Client;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type DaemonClient = Client<OwnedReadHalf, OwnedWriteHalf>;

/// Connects to the daemon and makes `calls` on the connection, which must finish within the timeout.
pub(crate) fn with_client<T>(
    calls: impl AsyncFnOnce(&mut DaemonClient) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let socket = paths::socket(&paths::runtime_dir()?);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the tokio runtime")?;
    runtime.block_on(async {
        let stream = match UnixStream::connect(&socket).await {
            Ok(stream) => stream,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                bail!("cloneyard daemon is not running ({})", socket.display())
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot connect to {}", socket.display()));
            }
        };
        let (reader, writer) = stream.into_split();
        let mut client = Client::new(reader, writer);
        tokio::time::timeout(TIMEOUT, calls(&mut client))
            .await
            .with_context(|| format!("the daemon did not answer within {} s", TIMEOUT.as_secs()))?
    })
}
