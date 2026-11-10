use cloneyard_api::rpc::Discover;

use crate::connection;

pub(crate) fn schema() -> anyhow::Result<()> {
    let document =
        connection::with_client(async |client| Ok(client.request::<Discover>(()).await?))?;
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}
