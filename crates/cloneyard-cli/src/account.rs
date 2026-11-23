use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::Path;

use anyhow::{Context as _, anyhow, bail};
use cloneyard_api::account::{Account, Add, AddError, AddParams, List, Provider};
use cloneyard_daemon::paths;

use crate::{claude, connection};

/// Logs in to Claude Code in a new directory, `accounts/<name>/claude` in the data dir, and adds it as account `name`; on a failure it removes the directory again.
pub(crate) fn add(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') {
        bail!(
            "invalid account name {name:?}: it names a directory, so it can't be empty, \".\", \"..\" or contain \"/\""
        );
    }
    let accounts =
        connection::with_client(async |client| Ok(client.request::<List>(()).await?))?.accounts;
    if accounts.iter().any(|account| account.name == name) {
        return Err(AddError::NameTaken {
            name: name.to_owned(),
        }
        .into());
    }
    let parent = paths::data_dir()?.join("accounts");
    let home = parent.join(name);
    let dir = home.join("claude");
    let dir_name = dir
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", dir.display()))?
        .to_owned();
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;
    // Not recursive, so a directory already there is never taken over.
    DirBuilder::new()
        .mode(0o700)
        .create(&home)
        .with_context(|| format!("cannot create {}", home.display()))?;
    let account = log_in_and_add(name, &dir, dir_name).map_err(|error| discard(&home, error))?;
    if accounts.is_empty() {
        println!("Hello there.");
    }
    println!("added account {} ({})", account.name, account.id);
    Ok(())
}

fn log_in_and_add(name: &str, dir: &Path, dir_name: String) -> anyhow::Result<Account> {
    DirBuilder::new()
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    if !claude::log_in(dir)? {
        bail!("claude auth login did not log in, so account {name} was not added");
    }
    claude::skip_onboarding(dir)?;
    let params = AddParams {
        name: name.to_owned(),
        provider: Provider::ClaudeCode,
        dir: dir_name,
    };
    connection::with_client(async |client| Ok(client.request::<Add>(params).await?))
}

fn discard(home: &Path, error: anyhow::Error) -> anyhow::Error {
    match fs::remove_dir_all(home) {
        Ok(()) => error,
        Err(removal) => anyhow!(
            "{error:#}; also cannot remove {}: {removal}",
            home.display()
        ),
    }
}

pub(crate) fn list() -> anyhow::Result<()> {
    let accounts =
        connection::with_client(async |client| Ok(client.request::<List>(()).await?))?.accounts;
    let width = accounts
        .iter()
        .map(|account| account.name.chars().count())
        .max()
        .unwrap_or_default();
    let provider_width = accounts
        .iter()
        .map(|account| account.provider.to_string().chars().count())
        .max()
        .unwrap_or_default();
    for account in &accounts {
        match &account.dir {
            Some(dir) => println!(
                "{:width$}  {:provider_width$}  {dir}",
                account.name, account.provider
            ),
            None => println!("{:width$}  {}", account.name, account.provider),
        }
    }
    Ok(())
}
