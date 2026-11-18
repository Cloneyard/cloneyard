use std::path::Path;

use cloneyard_store::Store;
use ulid::Ulid;

/// The state of the node that the handlers share: its id and its database.
pub(crate) struct Node {
    pub(crate) id: Ulid,
    pub(crate) store: Store,
}

impl Node {
    /// Opens the database at `database` and reads the node id from it, creating both if they are missing.
    pub(crate) async fn open(database: &Path) -> anyhow::Result<Self> {
        let store = Store::open(database).await?;
        let id = store.node_id().await?;
        Ok(Self { id, store })
    }
}
