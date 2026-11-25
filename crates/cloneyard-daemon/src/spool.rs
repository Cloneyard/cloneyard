//! A clone's spool: the directory in the clone's own one that its hooks write their events into, a file each, which the daemon reads in name order.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail};

/// The variable that tells `cy hook` the spool of its clone.
pub const VARIABLE: &str = "CLONEYARD_SPOOL";

/// Writes `payload`, the JSON that hook `event` got, into the spool at `dir`, as a file whose name holds the time, the pid and the event, so it sorts after the files of earlier events.
///
/// # Errors
///
/// When `event` isn't a word of letters and digits, or the file can't be written.
pub fn write(dir: &Path, event: &str, payload: &[u8]) -> anyhow::Result<PathBuf> {
    if event.is_empty() || !event.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        bail!("invalid hook name {event:?}");
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let name = format!("{nanos:020}-{}-{event}", process::id());
    // The daemon takes only `*.json`, so it never reads a file that is still being written.
    let temporary = dir.join(format!(".{name}.tmp"));
    let path = dir.join(format!("{name}.json"));
    let written = write_new(&temporary, payload).and_then(|()| {
        fs::rename(&temporary, &path).with_context(|| {
            format!(
                "cannot rename {} to {}",
                temporary.display(),
                path.display()
            )
        })
    });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map(|()| path)
}

fn write_new(path: &Path, payload: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(payload)
        .with_context(|| format!("cannot write {}", path.display()))
}
