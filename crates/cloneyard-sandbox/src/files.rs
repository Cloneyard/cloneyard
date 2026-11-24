//! Landlock: what a clone reads and writes.

use std::path::{Path, PathBuf};
use std::{fs, io};

use landlock::{
    ABI, Access as _, AccessFs, BitFlags, CompatLevel, Compatible as _, PathBeneath, PathFd,
    Ruleset, RulesetAttr as _, RulesetCreatedAttr as _, RulesetStatus, Scope,
};

use crate::{Error, Policy};

/// The newest ABI whose rights the rules use; ABI v9 adds `ResolveUnix`, which would also stop datagram sockets outside the writable paths.
const NEWEST: ABI = ABI::V6;

/// Whether the kernel has Landlock, enabled.
pub(crate) fn supported() -> bool {
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI::V1))
        .and_then(Ruleset::create)
        .is_ok()
}

/// Restricts this process and the ones it starts to `policy`; the first ABI is required, and what later ones add applies where the kernel has it.
pub(crate) fn restrict(policy: &Policy) -> Result<(), Error> {
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(ABI::V1))?
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(NEWEST))?
        .scope(Scope::from_all(NEWEST))?
        .create()?;
    for rule in rules(policy) {
        let metadata = match fs::metadata(&rule.path) {
            Ok(metadata) => metadata,
            Err(_) if !rule.required => continue,
            Err(error) => {
                return Err(Error::Open {
                    path: rule.path,
                    error,
                });
            }
        };
        let access = if metadata.is_dir() {
            rule.access
        } else {
            rule.access & AccessFs::from_file(NEWEST)
        };
        let fd = PathFd::new(&rule.path).map_err(|error| Error::Open {
            path: rule.path.clone(),
            error: io::Error::other(error),
        })?;
        ruleset = ruleset.add_rule(PathBeneath::new(fd, access))?;
    }
    if ruleset.restrict_self()?.ruleset == RulesetStatus::NotEnforced {
        return Err(Error::NoLandlock);
    }
    Ok(())
}

struct Rule {
    path: PathBuf,
    access: BitFlags<AccessFs>,
    /// Whether a missing path is an error, rather than a path that went away since it was listed.
    required: bool,
}

fn rules(policy: &Policy) -> Vec<Rule> {
    let read = AccessFs::from_read(NEWEST);
    let rule = |path: &Path, access, required| Rule {
        path: path.to_owned(),
        access,
        required,
    };
    let mut rules = Vec::new();
    if policy.hidden.is_empty() {
        rules.push(rule(Path::new("/"), read, true));
    } else {
        // Listing reveals only names, and without it on `/` a clone couldn't list the parents of its own directory.
        rules.push(rule(Path::new("/"), AccessFs::ReadDir.into(), true));
        let mut trees = Vec::new();
        around(Path::new("/"), &policy.hidden, &mut trees);
        rules.extend(trees.iter().map(|tree| rule(tree, read, false)));
    }
    rules.extend(policy.visible.iter().map(|path| rule(path, read, true)));
    rules.extend(
        policy
            .writable
            .iter()
            .map(|path| rule(path, AccessFs::from_all(NEWEST), true)),
    );
    rules
}

/// Adds to `trees` every entry of `dir` that neither is nor holds a hidden path, and goes down into those that hold one.
fn around(dir: &Path, hidden: &[PathBuf], trees: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(real) = fs::canonicalize(&path) else {
            continue;
        };
        if hidden.iter().any(|hidden| real.starts_with(hidden)) {
            continue;
        }
        if hidden.iter().any(|hidden| hidden.starts_with(&real)) {
            // A symlink to a parent of a hidden path stays closed; its target is walked where it is.
            if real == path {
                around(&path, hidden, trees);
            }
            continue;
        }
        trees.push(real);
    }
}
