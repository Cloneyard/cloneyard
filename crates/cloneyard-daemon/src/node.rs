use std::env;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cloneyard_store::Store;
use ulid::Ulid;

use crate::otlp::Tokens;
use crate::paths;
use crate::process::Processes;
use crate::sandbox::Sandboxing;
use crate::spool::{self, Spools};

/// The state of the node that the handlers share: its id, its database, its data dir, how it sandboxes its clones, their processes, the program their hooks run and where their telemetry goes.
pub(crate) struct Node {
    pub(crate) id: Ulid,
    pub(crate) store: Store,
    pub(crate) data_dir: PathBuf,
    pub(crate) sandboxing: Sandboxing,
    pub(crate) processes: Processes,
    /// The daemon's own binary, as `cy hook` writes a clone's hook events.
    pub(crate) program: String,
    pub(crate) spools: Spools,
    /// The port on `127.0.0.1` of the receiver of the clones' telemetry.
    pub(crate) telemetry_port: u16,
    pub(crate) tokens: Tokens,
}

impl Node {
    /// Opens the database in `data_dir` and reads the node id from it, creating both if they are missing, marks the clones that were running as lost and takes the hook events their spools hold, for clones whose telemetry goes to `telemetry_port`; call it inside a tokio runtime.
    pub(crate) async fn open(
        data_dir: &Path,
        sandboxing: Sandboxing,
        telemetry_port: u16,
    ) -> anyhow::Result<Self> {
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
        spool::pass_all(&store, data_dir).await;
        let spools = Spools::new()?;
        Ok(Self {
            id,
            store,
            data_dir: data_dir.to_owned(),
            sandboxing,
            processes: Processes::default(),
            program,
            spools,
            telemetry_port,
            tokens: Tokens::default(),
        })
    }
}
