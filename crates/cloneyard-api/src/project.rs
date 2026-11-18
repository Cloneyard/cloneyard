use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::{ErrorCode, Method, MethodError, NoError};

pub struct Add;

impl Method for Add {
    const NAME: &'static str = "project.add";
    const SUMMARY: Option<&'static str> =
        Some("Adds the git repo at a path as a project with that one repo");
    type Params = AddParams;
    type Result = Project;
    type Error = AddError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AddParams {
    /// An absolute path in the repo's working tree, such as its top level or a directory inside it.
    pub path: String,
    /// The name of the project, or the name of the repo's directory when left out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum AddError {
    #[error("{path} is not an absolute path")]
    PathNotAbsolute { path: String },
    #[error("{path} is not in a git working tree")]
    NotInWorkTree { path: String },
    #[error("{path} is already in project {project}")]
    RepoTaken {
        /// The top level of the repo.
        path: String,
        /// The name of the project the repo is in.
        project: String,
    },
    #[error("project {name} already exists")]
    NameTaken { name: String },
}

impl MethodError for AddError {
    // project.* has the codes 1000 to 1099.
    const ERRORS: &'static [ErrorCode] = &[
        ErrorCode {
            variant: "PathNotAbsolute",
            code: 1000,
            message: "Path is not absolute",
        },
        ErrorCode {
            variant: "NotInWorkTree",
            code: 1001,
            message: "Path is not in a git working tree",
        },
        ErrorCode {
            variant: "RepoTaken",
            code: 1002,
            message: "Repo is already in a project",
        },
        ErrorCode {
            variant: "NameTaken",
            code: 1003,
            message: "Project name is taken",
        },
    ];
}

pub struct List;

impl Method for List {
    const NAME: &'static str = "project.list";
    const SUMMARY: Option<&'static str> =
        Some("Returns the projects with their repos on this node");
    type Params = ();
    type Result = ListResult;
    type Error = NoError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListResult {
    /// Sorted by name.
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Project {
    #[schemars(schema_with = "crate::ulid_schema")]
    pub id: Ulid,
    /// Unique among the projects.
    pub name: String,
    /// The repos with a path on this node, sorted by name.
    pub repos: Vec<Repo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Repo {
    #[schemars(schema_with = "crate::ulid_schema")]
    pub id: Ulid,
    /// The name of the repo's directory, unique within its project.
    pub name: String,
    /// The top level of the repo's working tree on this node.
    pub path: String,
}
