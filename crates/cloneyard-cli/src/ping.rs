use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use cloneyard_daemon::paths;
use cloneyard_daemon::system::{self, Version};
use cloneyard_rpc::Client;
use tokio::net::UnixStream;

const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn run() -> anyhow::Result<()> {
    let socket = paths::socket(&paths::runtime_dir()?);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the tokio runtime")?;
    let (version, round_trip) = runtime.block_on(async {
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
        tokio::time::timeout(TIMEOUT, async {
            let version: Version = client.call(system::VERSION, ()).await?;
            let start = Instant::now();
            let _: String = client.call(system::PING, ()).await?;
            anyhow::Ok((version, start.elapsed()))
        })
        .await
        .with_context(|| format!("the daemon did not answer within {} s", TIMEOUT.as_secs()))?
    })?;
    println!(
        "cloneyard {}, round trip {:.2} ms",
        version.version,
        round_trip.as_secs_f64() * 1000.0
    );
    Ok(())
}
