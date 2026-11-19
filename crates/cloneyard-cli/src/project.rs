use std::path::{self, Path};

use anyhow::Context as _;
use cloneyard_api::project::{Add, AddParams, List};

use crate::connection;

pub(crate) fn add(path: &Path, name: Option<String>) -> anyhow::Result<()> {
    let absolute =
        path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))?;
    let path = absolute
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", absolute.display()))?
        .to_owned();
    let project = connection::with_client(async |client| {
        Ok(client.request::<Add>(AddParams { path, name }).await?)
    })?;
    println!("added project {} ({})", project.name, project.id);
    Ok(())
}

pub(crate) fn list() -> anyhow::Result<()> {
    let projects =
        connection::with_client(async |client| Ok(client.request::<List>(()).await?))?.projects;
    let width = projects
        .iter()
        .map(|project| project.name.chars().count())
        .max()
        .unwrap_or_default();
    for project in &projects {
        if project.repos.is_empty() {
            println!("{}", project.name);
        }
        for repo in &project.repos {
            println!("{:width$}  {}", project.name, repo.path);
        }
    }
    Ok(())
}
