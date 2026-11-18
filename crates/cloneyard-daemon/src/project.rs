use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context as _;
use cloneyard_api::project::{AddError, AddParams, ListResult, Project};
use tokio::process::Command;

use crate::node::Node;

/// Adds the repo whose working tree holds the path, at its top level with symlinks resolved.
pub(crate) async fn add(
    node: &Node,
    params: AddParams,
) -> anyhow::Result<Result<Project, AddError>> {
    let AddParams { path, name } = params;
    if !Path::new(&path).is_absolute() {
        return Ok(Err(AddError::PathNotAbsolute { path }));
    }
    let Some(canonical) = top_level(Path::new(&path)).await? else {
        return Ok(Err(AddError::NotInWorkTree { path }));
    };
    let top_level = canonical
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", canonical.display()))?;
    // Only / has no file name.
    let repo = canonical
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(top_level);
    let name = name.as_deref().unwrap_or(repo);
    node.store.add_project(node.id, name, repo, top_level).await
}

pub(crate) async fn list(node: &Node) -> anyhow::Result<ListResult> {
    Ok(ListResult {
        projects: node.store.projects(node.id).await?,
    })
}

/// The top level of the git working tree that holds `path`, or `None` when no working tree does.
async fn top_level(path: &Path) -> anyhow::Result<Option<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null())
        .output()
        .await
        .with_context(|| format!("cannot run git in {}", path.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let printed = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
    let top_level = Path::new(OsStr::from_bytes(printed));
    let canonical = tokio::fs::canonicalize(top_level)
        .await
        .with_context(|| format!("cannot resolve {}", top_level.display()))?;
    Ok(Some(canonical))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use cloneyard_api::project::{Add, AddError, AddParams, List, ListResult, Project};
    use cloneyard_rpc::{ClientError, Error};
    use tempfile::TempDir;

    use crate::router::tests::{TestClient, connect, open_node};

    fn repo(path: &Path) -> PathBuf {
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
        fs::canonicalize(path).unwrap()
    }

    fn params(path: &Path, name: Option<&str>) -> AddParams {
        AddParams {
            path: path.to_str().unwrap().to_owned(),
            name: name.map(str::to_owned),
        }
    }

    async fn client(dir: &TempDir) -> TestClient {
        connect(open_node(dir).await)
    }

    async fn add(client: &mut TestClient, path: &Path) -> Project {
        client.request::<Add>(params(path, None)).await.unwrap()
    }

    async fn error_of(client: &mut TestClient, params: AddParams) -> AddError {
        match client.request::<Add>(params).await {
            Err(ClientError::Method(error)) => error,
            other => panic!("expected an error of project.add, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adds_the_repo_of_a_subdirectory_named_after_its_top_level() {
        let dir = TempDir::new().unwrap();
        let top_level = repo(&dir.path().join("app"));
        fs::create_dir(top_level.join("src")).unwrap();
        let mut client = client(&dir).await;

        let project = add(&mut client, &top_level.join("src")).await;

        let [repo] = project.repos.as_slice() else {
            panic!("{project:?}");
        };
        assert_eq!(project.name, "app");
        assert_eq!(repo.name, "app");
        assert_eq!(Path::new(&repo.path), top_level);
    }

    #[tokio::test]
    async fn names_the_project_as_asked_and_the_repo_after_its_directory() {
        let dir = TempDir::new().unwrap();
        let top_level = repo(&dir.path().join("app"));
        let mut client = client(&dir).await;

        let project = client
            .request::<Add>(params(&top_level, Some("web")))
            .await
            .unwrap();

        assert_eq!(project.name, "web");
        assert_eq!(project.repos[0].name, "app");
    }

    #[tokio::test]
    async fn lists_the_projects_it_added() {
        let dir = TempDir::new().unwrap();
        let mut client = client(&dir).await;
        let web = add(&mut client, &repo(&dir.path().join("web"))).await;
        let api = add(&mut client, &repo(&dir.path().join("api"))).await;

        let listed = client.request::<List>(()).await.unwrap();

        assert_eq!(
            listed,
            ListResult {
                projects: vec![api, web]
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_relative_path() {
        let dir = TempDir::new().unwrap();
        let mut client = client(&dir).await;

        let error = error_of(&mut client, params(Path::new("app"), None)).await;

        assert_eq!(
            error,
            AddError::PathNotAbsolute {
                path: "app".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_path_outside_a_git_working_tree() {
        let dir = TempDir::new().unwrap();
        let notes = dir.path().join("notes");
        fs::create_dir(&notes).unwrap();
        let mut client = client(&dir).await;

        for path in [notes.clone(), notes.join("missing")] {
            let error = error_of(&mut client, params(&path, None)).await;

            assert_eq!(
                error,
                AddError::NotInWorkTree {
                    path: path.to_str().unwrap().to_owned()
                }
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_repo_already_in_a_project_when_reached_through_a_symlink() {
        let dir = TempDir::new().unwrap();
        let top_level = repo(&dir.path().join("app"));
        let link = dir.path().join("link");
        symlink(&top_level, &link).unwrap();
        let mut client = client(&dir).await;
        add(&mut client, &top_level).await;

        let error = error_of(&mut client, params(&link, Some("other"))).await;

        assert_eq!(
            error,
            AddError::RepoTaken {
                path: top_level.to_str().unwrap().to_owned(),
                project: "app".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_name_that_another_project_has() {
        let dir = TempDir::new().unwrap();
        let first = repo(&dir.path().join("code").join("app"));
        let second = repo(&dir.path().join("work").join("app"));
        let mut client = client(&dir).await;
        add(&mut client, &first).await;

        let error = error_of(&mut client, params(&second, None)).await;

        assert_eq!(
            error,
            AddError::NameTaken {
                name: "app".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn fails_as_an_internal_error_on_a_top_level_that_is_not_utf_8() {
        let dir = TempDir::new().unwrap();
        let top_level = repo(&dir.path().join(OsStr::from_bytes(b"caf\xe9")));
        let link = dir.path().join("link");
        symlink(&top_level, &link).unwrap();
        let mut client = client(&dir).await;

        let result = client.request::<Add>(params(&link, None)).await;

        match result {
            Err(ClientError::Rpc(error)) => assert_eq!(
                error,
                Error::internal(format!("{} is not valid UTF-8", top_level.display()))
            ),
            other => panic!("expected an internal error, got {other:?}"),
        }
    }
}
