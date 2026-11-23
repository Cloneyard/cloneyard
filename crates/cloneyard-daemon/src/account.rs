use std::io;
use std::path::Path;

use anyhow::Context as _;
use cloneyard_api::account::{Account, AddError, AddParams, ListResult};

use crate::node::Node;

/// Records the account with its directory, with symlinks resolved; the login in it is the caller's.
pub(crate) async fn add(
    node: &Node,
    params: AddParams,
) -> anyhow::Result<Result<Account, AddError>> {
    let AddParams {
        name,
        provider,
        dir,
    } = params;
    if !Path::new(&dir).is_absolute() {
        return Ok(Err(AddError::DirNotAbsolute { dir }));
    }
    let canonical = match tokio::fs::canonicalize(&dir).await {
        Ok(canonical) => canonical,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(Err(AddError::NotADirectory { dir }));
        }
        Err(error) => return Err(error).with_context(|| format!("cannot resolve {dir}")),
    };
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .with_context(|| format!("cannot read {}", canonical.display()))?;
    if !metadata.is_dir() {
        return Ok(Err(AddError::NotADirectory { dir }));
    }
    let canonical = canonical
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", canonical.display()))?;
    node.store
        .add_account(node.id, &name, provider, canonical)
        .await
}

pub(crate) async fn list(node: &Node) -> anyhow::Result<ListResult> {
    Ok(ListResult {
        accounts: node.store.accounts(node.id).await?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use cloneyard_api::account::{Account, Add, AddError, AddParams, List, ListResult, Provider};
    use cloneyard_rpc::ClientError;
    use tempfile::TempDir;

    use crate::router::tests::{TestClient, connect, open_node};

    fn params(name: &str, dir: &Path) -> AddParams {
        AddParams {
            name: name.to_owned(),
            provider: Provider::ClaudeCode,
            dir: dir.to_str().unwrap().to_owned(),
        }
    }

    /// A new directory at `path`, given as its canonical path.
    fn new_dir(path: &Path) -> String {
        fs::create_dir_all(path).unwrap();
        fs::canonicalize(path).unwrap().to_str().unwrap().to_owned()
    }

    async fn client(dir: &TempDir) -> TestClient {
        connect(open_node(dir).await)
    }

    async fn add(client: &mut TestClient, name: &str, dir: &Path) -> Account {
        client.request::<Add>(params(name, dir)).await.unwrap()
    }

    async fn error_of(client: &mut TestClient, params: AddParams) -> AddError {
        match client.request::<Add>(params).await {
            Err(ClientError::Method(error)) => error,
            other => panic!("expected an error of account.add, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adds_an_account_at_its_directory_with_symlinks_resolved() {
        let dir = TempDir::new().unwrap();
        let claude = new_dir(&dir.path().join("accounts/personal/claude"));
        let link = dir.path().join("link");
        symlink(&claude, &link).unwrap();
        let mut client = client(&dir).await;

        let account = add(&mut client, "personal", &link).await;

        assert_eq!(account.name, "personal");
        assert_eq!(account.provider, Provider::ClaudeCode);
        assert_eq!(account.dir, Some(claude));
    }

    #[tokio::test]
    async fn lists_the_accounts_it_added() {
        let dir = TempDir::new().unwrap();
        let mut client = client(&dir).await;
        let work = add(
            &mut client,
            "work",
            Path::new(&new_dir(&dir.path().join("w"))),
        )
        .await;
        let personal = add(
            &mut client,
            "personal",
            Path::new(&new_dir(&dir.path().join("p"))),
        )
        .await;

        let listed = client.request::<List>(()).await.unwrap();

        assert_eq!(
            listed,
            ListResult {
                accounts: vec![personal, work]
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_relative_directory() {
        let dir = TempDir::new().unwrap();
        let mut client = client(&dir).await;

        let error = error_of(&mut client, params("personal", Path::new("claude"))).await;

        assert_eq!(
            error,
            AddError::DirNotAbsolute {
                dir: "claude".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_path_that_is_not_a_directory() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("notes");
        fs::write(&file, "").unwrap();
        let mut client = client(&dir).await;

        for path in [
            dir.path().join("missing"),
            file.clone(),
            file.join("claude"),
        ] {
            let error = error_of(&mut client, params("personal", &path)).await;

            assert_eq!(
                error,
                AddError::NotADirectory {
                    dir: path.to_str().unwrap().to_owned()
                }
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_directory_that_another_account_has_when_reached_through_a_symlink() {
        let dir = TempDir::new().unwrap();
        let claude = new_dir(&dir.path().join("claude"));
        let link = dir.path().join("link");
        symlink(&claude, &link).unwrap();
        let mut client = client(&dir).await;
        add(&mut client, "personal", Path::new(&claude)).await;

        let error = error_of(&mut client, params("work", &link)).await;

        assert_eq!(
            error,
            AddError::DirTaken {
                dir: claude,
                account: "personal".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn refuses_a_name_that_another_account_has() {
        let dir = TempDir::new().unwrap();
        let mut client = client(&dir).await;
        add(
            &mut client,
            "personal",
            Path::new(&new_dir(&dir.path().join("first"))),
        )
        .await;

        let error = error_of(
            &mut client,
            params("personal", Path::new(&new_dir(&dir.path().join("second")))),
        )
        .await;

        assert_eq!(
            error,
            AddError::NameTaken {
                name: "personal".to_owned()
            }
        );
    }
}
