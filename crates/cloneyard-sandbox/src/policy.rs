use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// What a clone may read and write: everything but `hidden` to read, unless it is in `visible` or `writable`, and only `writable` to write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Unchecked")]
pub struct Policy {
    pub(crate) hidden: Vec<PathBuf>,
    pub(crate) visible: Vec<PathBuf>,
    pub(crate) writable: Vec<PathBuf>,
}

#[derive(Deserialize)]
struct Unchecked {
    hidden: Vec<PathBuf>,
    visible: Vec<PathBuf>,
    writable: Vec<PathBuf>,
}

impl TryFrom<Unchecked> for Policy {
    type Error = Error;

    fn try_from(paths: Unchecked) -> Result<Self, Error> {
        Self::new(paths.hidden, paths.visible, paths.writable)
    }
}

impl Policy {
    /// A policy with its paths resolved through the symlinks on their way; a path may not exist yet.
    ///
    /// # Errors
    ///
    /// When a path isn't absolute or isn't UTF-8, or a hidden path lies inside a visible or writable one, since Landlock can't take back part of a tree it granted.
    pub fn new(
        hidden: Vec<PathBuf>,
        visible: Vec<PathBuf>,
        writable: Vec<PathBuf>,
    ) -> Result<Self, Error> {
        let [hidden, visible, writable] = [hidden, visible, writable].map(|paths| {
            paths
                .into_iter()
                .map(|path| {
                    if path.is_absolute() && path.to_str().is_some() {
                        Ok(resolve(&path))
                    } else {
                        Err(Error::NotAbsolute(path))
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        });
        let policy = Self {
            hidden: hidden?,
            visible: visible?,
            writable: writable?,
        };
        for hidden in &policy.hidden {
            if let Some(granted) = policy
                .visible
                .iter()
                .chain(&policy.writable)
                .find(|granted| hidden.starts_with(granted) && hidden != *granted)
            {
                return Err(Error::HiddenInside {
                    hidden: hidden.clone(),
                    granted: granted.clone(),
                });
            }
        }
        Ok(policy)
    }

    #[must_use]
    pub fn hidden(&self) -> &[PathBuf] {
        &self.hidden
    }

    #[must_use]
    pub fn visible(&self) -> &[PathBuf] {
        &self.visible
    }

    #[must_use]
    pub fn writable(&self) -> &[PathBuf] {
        &self.writable
    }
}

/// `path` with its longest existing ancestor canonicalized.
fn resolve(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(real) = fs::canonicalize(existing) {
            return missing
                .iter()
                .rev()
                .fold(real, |path, name| path.join(name));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => return path.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::Policy;

    fn paths(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_hidden_path_inside_a_writable_one_is_an_error_that_names_both() {
        let error = Policy::new(paths(&["/tmp/app"]), Vec::new(), paths(&["/tmp"])).unwrap_err();

        assert_eq!(
            error.to_string(),
            "cannot hide /tmp/app inside /tmp, which the clone can read"
        );
    }

    #[test]
    fn a_path_visible_inside_a_hidden_one_or_hidden_and_writable_at_once_is_fine() {
        let policy = Policy::new(
            paths(&["/data", "/home/me/.claude"]),
            paths(&["/data/repo/.git/objects"]),
            paths(&["/data/clones/1", "/home/me/.claude"]),
        );

        assert!(policy.is_ok(), "{policy:?}");
    }

    #[test]
    fn a_relative_path_is_an_error() {
        let error = Policy::new(paths(&["app/.env"]), Vec::new(), Vec::new()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "app/.env is not an absolute path in UTF-8"
        );
    }

    #[test]
    fn paths_are_resolved_through_symlinks_even_when_they_do_not_exist_yet() {
        let dir = TempDir::new().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap().join("real");
        fs::create_dir(&real).unwrap();
        symlink(&real, dir.path().join("link")).unwrap();

        let policy = Policy::new(
            vec![dir.path().join("link/.ssh")],
            Vec::new(),
            vec![dir.path().join("link/work")],
        )
        .unwrap();

        assert_eq!(policy.hidden, [real.join(".ssh")]);
        assert_eq!(policy.writable, [real.join("work")]);
    }

    #[test]
    fn a_policy_read_back_from_json_is_checked_again() {
        let json = r#"{"hidden":["/tmp/app"],"visible":[],"writable":["/tmp"]}"#;

        let error = serde_json::from_str::<Policy>(json).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("cannot hide /tmp/app inside /tmp"),
            "{error}"
        );
    }
}
