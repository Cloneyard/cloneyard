//! What the daemon puts in a clone's sandbox, and whether it sandboxes clones at all.

use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use cloneyard_sandbox::Policy;
use tokio::process::Command;

use crate::node::Node;

/// The program that enters the sandbox: the daemon's own binary, through `cy sandbox`.
pub(crate) const ENTER: &str = "/proc/self/exe";

/// The paths in a home that hold keys, tokens or logins.
const SECRETS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".docker",
    ".kube",
    ".netrc",
    ".git-credentials",
    ".config/gh",
    ".config/hub",
    ".config/gcloud",
    ".local/share/keyrings",
    ".password-store",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".claude",
    ".claude.json",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".npmrc",
    ".pypirc",
    ".composer/auth.json",
    ".config/composer/auth.json",
];

/// How the daemon starts its clones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sandboxing {
    /// In the sandbox.
    On,
    /// Without it, as `node.toml` says `unsandboxed = true`.
    Off,
    /// Not at all, as the kernel has no Landlock and `node.toml`, at `config_file`, doesn't say `unsandboxed = true`.
    Unavailable { config_file: PathBuf },
}

impl Sandboxing {
    /// `Off` when `unsandboxed`, otherwise `On` on a kernel with Landlock and `Unavailable` on one without.
    #[must_use]
    pub fn new(unsandboxed: bool, config_file: &Path) -> Self {
        if unsandboxed {
            Self::Off
        } else if cloneyard_sandbox::supported() {
            Self::On
        } else {
            Self::Unavailable {
                config_file: config_file.to_owned(),
            }
        }
    }
}

/// The sandbox of a clone in `clone_dir` on the account in `account_dir`.
pub(crate) async fn policy(
    node: &Node,
    home: Option<&Path>,
    clone_dir: &Path,
    account_dir: &Path,
) -> anyhow::Result<Policy> {
    let Paths {
        hidden,
        visible,
        writable,
    } = paths(node, home, clone_dir, account_dir).await?;
    Ok(Policy::new(hidden, visible, writable)?)
}

#[derive(Debug)]
struct Paths {
    hidden: Vec<PathBuf>,
    visible: Vec<PathBuf>,
    writable: Vec<PathBuf>,
}

/// A clone can't read the secrets in `home`, the data dir, the accounts or the repos of the node, apart from its own directory, its account's, the objects its checkout borrows and the daemon's binary, and it writes only there, in the caches, `/tmp` and `/dev`.
async fn paths(
    node: &Node,
    home: Option<&Path>,
    clone_dir: &Path,
    account_dir: &Path,
) -> anyhow::Result<Paths> {
    let mut hidden: Vec<PathBuf> = home
        .into_iter()
        .flat_map(|home| SECRETS.iter().map(|secret| home.join(secret)))
        .collect();
    hidden.push(node.data_dir.clone());
    hidden.extend(
        node.store
            .accounts(node.id)
            .await?
            .into_iter()
            .filter_map(|account| account.dir.map(PathBuf::from)),
    );
    hidden.extend(
        node.store
            .projects(node.id)
            .await?
            .into_iter()
            .flat_map(|project| project.repos)
            .map(|repo| PathBuf::from(repo.path)),
    );
    let mut visible = borrowed_objects(&clone_dir.join("checkout"))?;
    // The hooks run it, and it may sit in the `target/` of a repo that is hidden.
    visible.push(PathBuf::from(&node.program));
    let writable = vec![
        clone_dir.to_owned(),
        account_dir.to_owned(),
        caches(&node.data_dir)?,
        PathBuf::from("/tmp"),
        PathBuf::from("/dev"),
    ];
    Ok(Paths {
        hidden,
        visible,
        writable,
    })
}

/// The object directories the checkout reads through `objects/info/alternates`.
fn borrowed_objects(checkout: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let path = checkout.join(".git/objects/info/alternates");
    let alternates =
        fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(alternates
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(PathBuf::from)
        .collect())
}

/// `caches` in the data dir, which the clones share, so none writes a cache that runs outside the sandbox.
fn caches(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let caches = data_dir.join("caches");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&caches)
        .with_context(|| format!("cannot create {}", caches.display()))?;
    Ok(caches)
}

/// Points the build tools of `command` at the caches in `data_dir`.
pub(crate) fn use_caches(command: &mut Command, data_dir: &Path) {
    let caches = data_dir.join("caches");
    command
        .env("XDG_CACHE_HOME", &caches)
        .env("CARGO_HOME", caches.join("cargo"))
        .env("npm_config_cache", caches.join("npm"))
        .env("GOMODCACHE", caches.join("go-mod"));
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use cloneyard_api::account::Provider;
    use tempfile::TempDir;

    use super::paths;
    use crate::router::tests::open_node;

    #[tokio::test]
    async fn a_clone_reads_neither_the_secrets_nor_the_data_dir_accounts_or_repos_but_its_own() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        for (name, path) in [("app", "/code/app"), ("web", "/code/web")] {
            node.store
                .add_project(node.id, name, name, path)
                .await
                .unwrap()
                .unwrap();
        }
        for (name, path) in [
            ("personal", "/accounts/personal"),
            ("work", "/accounts/work"),
        ] {
            node.store
                .add_account(node.id, name, Provider::ClaudeCode, path)
                .await
                .unwrap()
                .unwrap();
        }
        let clone_dir = dir.path().join("clones/01K");
        let info = clone_dir.join("checkout/.git/objects/info");
        fs::create_dir_all(&info).unwrap();
        fs::write(info.join("alternates"), "/code/app/.git/objects\n").unwrap();

        let paths = paths(
            &node,
            Some(Path::new("/home/me")),
            &clone_dir,
            Path::new("/accounts/personal"),
        )
        .await
        .unwrap();

        let hidden: Vec<&str> = paths
            .hidden
            .iter()
            .map(|path| path.to_str().unwrap())
            .collect();
        for path in [
            "/home/me/.ssh",
            "/home/me/.config/gh",
            "/home/me/.claude",
            "/accounts/personal",
            "/accounts/work",
            "/code/app",
            "/code/web",
            dir.path().to_str().unwrap(),
        ] {
            assert!(hidden.contains(&path), "{path} is not hidden: {hidden:?}");
        }
        assert_eq!(
            paths.visible,
            [
                PathBuf::from("/code/app/.git/objects"),
                PathBuf::from(&node.program)
            ]
        );
        assert_eq!(
            paths.writable,
            [
                clone_dir,
                PathBuf::from("/accounts/personal"),
                dir.path().join("caches"),
                PathBuf::from("/tmp"),
                PathBuf::from("/dev"),
            ]
        );
    }

    #[tokio::test]
    async fn a_checkout_without_alternates_names_the_file() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        let clone_dir = dir.path().join("clones/01K");

        let error = paths(&node, None, &clone_dir, Path::new("/accounts/personal"))
            .await
            .unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            format!(
                "cannot read {}: No such file or directory (os error 2)",
                clone_dir
                    .join("checkout/.git/objects/info/alternates")
                    .display()
            )
        );
    }
}
