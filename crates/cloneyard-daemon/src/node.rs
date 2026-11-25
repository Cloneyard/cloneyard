use std::env;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cloneyard_store::Store;
use ulid::Ulid;

use crate::paths;
use crate::process::Processes;
use crate::sandbox::Sandboxing;

/// The state of the node that the handlers share: its id, its database, its data dir, how it sandboxes its clones, their processes and the program their hooks run.
pub(crate) struct Node {
    pub(crate) id: Ulid,
    pub(crate) store: Store,
    pub(crate) data_dir: PathBuf,
    pub(crate) sandboxing: Sandboxing,
    pub(crate) processes: Processes,
    /// The daemon's own binary, as `cy hook` writes a clone's hook events.
    pub(crate) program: String,
}

impl Node {
    /// Opens the database in `data_dir` and reads the node id from it, creating both if they are missing, and marks the clones that were working as lost.
    pub(crate) async fn open(data_dir: &Path, sandboxing: Sandboxing) -> anyhow::Result<Self> {
        let program = env::current_exe().context("cannot find the daemon's own binary")?;
        let program = program
            .to_str()
            .with_context(|| format!("{} is not valid UTF-8", program.display()))?
            .to_owned();
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
            sandboxing,
            processes: Processes::default(),
            program,
        })
    }
}
