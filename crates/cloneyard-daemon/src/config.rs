use std::error::Error as _;
use std::iter;
use std::path::Path;
use std::{fs, io};

use anyhow::Context as _;
use serde::{Deserialize, Deserializer};
use tracing_subscriber::EnvFilter;

/// The node's settings from `node.toml`.
#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub log: Log,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Log {
    /// A `tracing` filter, such as `info` or `cloneyard_rpc=debug`.
    #[serde(deserialize_with = "filter")]
    pub level: String,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
        }
    }
}

fn filter<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let level = String::deserialize(deserializer)?;
    parse_filter(&level).map_err(serde::de::Error::custom)?;
    Ok(level)
}

// `ParseError` already writes its source into its own message, so `{:#}` would print it twice.
pub(crate) fn parse_filter(directives: &str) -> anyhow::Result<EnvFilter> {
    EnvFilter::try_new(directives).map_err(|error| {
        let causes = iter::successors(
            error.source().and_then(|source| source.source()),
            |&cause| cause.source(),
        );
        let message = iter::once(error.to_string())
            .chain(causes.map(ToString::to_string))
            .collect::<Vec<_>>()
            .join(": ");
        anyhow::anyhow!(message)
    })
}

impl Config {
    /// Reads `path`; a missing file gives the defaults.
    ///
    /// # Errors
    ///
    /// When the file can't be read or has an unknown key or a bad value.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{Config, Log};

    fn load(text: &str) -> anyhow::Result<Config> {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("node.toml");
        fs::write(&path, text).unwrap();
        Config::load(&path)
    }

    fn error_of(text: &str) -> String {
        format!("{:#}", load(text).unwrap_err())
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = TempDir::new().unwrap();
        let config = Config::load(&dir.path().join("node.toml")).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn an_empty_file_gives_the_defaults() {
        assert_eq!(load("").unwrap(), Config::default());
    }

    #[test]
    fn reads_the_log_filter() {
        let config = load("[log]\nlevel = \"warn,cloneyard_rpc=debug\"\n").unwrap();
        assert_eq!(
            config.log,
            Log {
                level: "warn,cloneyard_rpc=debug".to_owned()
            }
        );
    }

    #[test]
    fn rejects_an_unknown_key_with_the_path_and_line() {
        let error = error_of("[log]\nlevel = \"info\"\ncolour = \"red\"\n");
        assert!(error.contains("node.toml"), "{error}");
        assert!(error.contains("line 3"), "{error}");
        assert!(error.contains("unknown field `colour`"), "{error}");
    }

    #[test]
    fn rejects_a_bad_log_filter() {
        let error = error_of("[log]\nlevel = \"cloneyard=loud\"\n");
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn rejects_a_bad_field_filter_with_the_regex_error() {
        let error = error_of("[log]\nlevel = \"[{x=/(}]\"\n");
        assert!(
            error.contains(concat!(
                "invalid field filter: error building NFA: error parsing regex: ",
                "regex parse error:\n    /(\n     ^\nerror: unclosed group\n",
            )),
            "{error}"
        );
    }
}
