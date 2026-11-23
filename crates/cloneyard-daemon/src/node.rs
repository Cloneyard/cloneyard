use std::path::{Path, PathBuf};

use cloneyard_store::Store;
use ulid::Ulid;

use crate::paths;
use crate::process::Processes;

/// The state of the node that the handlers share: its id, its database, its data dir and the processes of its clones.
pub(crate) struct Node {
    pub(crate) id: Ulid,
    pub(crate) store: Store,
    pub(crate) data_dir: PathBuf,
    pub(crate) processes: Processes,
}

impl Node {
    /// Opens the database in `data_dir` and reads the node id from it, creating both if they are missing, and marks the clones that were working as lost.
    pub(crate) async fn open(data_dir: &Path) -> anyhow::Result<Self> {
        let store = Store::open(&paths::database(data_dir)).await?;
        let id = store.node_id().await?;
        let lost = store
            .lose_clones(id, "lost while the daemon was down")
            .await?;
        if !lost.is_empty() {
            let handles: Vec<String> = lost.iter().map(ToString::to_string).collect();
            tracing::warn!(
                clones = handles.join(", "),
                "clones lost while the daemon was down"
            );
        }
        Ok(Self {
            id,
            store,
            data_dir: data_dir.to_owned(),
            processes: Processes::default(),
        })
    }
}
