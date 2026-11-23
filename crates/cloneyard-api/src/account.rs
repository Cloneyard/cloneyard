use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{ErrorCode, Method, MethodError, NoError};

pub struct Add;

impl Method for Add {
    const NAME: &'static str = "account.add";
    const SUMMARY: Option<&'static str> =
        Some("Adds an account that is logged in to its provider in a directory on this node");
    type Params = AddParams;
    type Result = Account;
    type Error = AddError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AddParams {
    /// The name of the account.
    pub name: String,
    pub provider: Provider,
    /// The absolute path of the directory that holds the account's login on this node.
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum AddError {
    #[error("{dir} is not an absolute path")]
    DirNotAbsolute { dir: String },
    #[error("{dir} is not a directory")]
    NotADirectory { dir: String },
    #[error("{dir} is already the directory of account {account}")]
    DirTaken {
        /// The directory with symlinks resolved.
        dir: String,
        /// The name of the account that has the directory.
        account: String,
    },
    #[error("account {name} already exists")]
    NameTaken { name: String },
}

impl MethodError for AddError {
    // account.* has the codes 1100 to 1199.
    const ERRORS: &'static [ErrorCode] = &[
        ErrorCode {
            variant: "DirNotAbsolute",
            code: 1100,
            message: "Directory is not an absolute path",
        },
        ErrorCode {
            variant: "NotADirectory",
            code: 1101,
            message: "Path is not a directory",
        },
        ErrorCode {
            variant: "DirTaken",
            code: 1102,
            message: "Directory belongs to another account",
        },
        ErrorCode {
            variant: "NameTaken",
            code: 1103,
            message: "Account name is taken",
        },
    ];
}

pub struct List;

impl Method for List {
    const NAME: &'static str = "account.list";
    const SUMMARY: Option<&'static str> =
        Some("Returns the accounts with their directories on this node");
    type Params = ();
    type Result = ListResult;
    type Error = NoError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "AccountListResult")]
pub struct ListResult {
    /// Sorted by name.
    pub accounts: Vec<Account>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Account {
    #[schemars(schema_with = "crate::ulid_schema")]
    pub id: Ulid,
    /// Unique among the accounts.
    pub name: String,
    pub provider: Provider,
    /// The directory that holds the account's login on this node, or null when it has none here.
    pub dir: Option<String>,
}

/// The coding agent CLI that an account logs in to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Provider {
    /// Claude Code, which keeps each login in a config directory of its own.
    ClaudeCode,
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClaudeCode => f.pad("claude-code"),
        }
    }
}
