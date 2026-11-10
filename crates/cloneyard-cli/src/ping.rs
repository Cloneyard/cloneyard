use std::time::Instant;

use cloneyard_api::system::{Ping, Version};

use crate::connection;

pub(crate) fn run() -> anyhow::Result<()> {
    let (version, ping, round_trip) = connection::with_client(async |client| {
        let version = client.request::<Version>(()).await?;
        let start = Instant::now();
        let ping = client.request::<Ping>(()).await?;
        anyhow::Ok((version, ping, start.elapsed()))
    })?;
    println!(
        "cloneyard {}, node {}, round trip {:.2} ms",
        version.version,
        ping.node_id,
        round_trip.as_secs_f64() * 1000.0
    );
    Ok(())
}
