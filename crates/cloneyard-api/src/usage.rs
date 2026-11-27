use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ErrorCode, Method, MethodError};

pub struct Tools;

impl Method for Tools {
    const NAME: &'static str = "usage.tools";
    const SUMMARY: Option<&'static str> = Some(
        "Returns the tool calls of the clones on this node since a point in time, per tool and what the calls were about",
    );
    type Params = ToolsParams;
    type Result = ToolsResult;
    type Error = ToolsError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolsParams {
    /// The start of the period, in milliseconds since the Unix epoch.
    pub since: i64,
    /// The name of the project whose clones count, or every project when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "UsageToolsResult")]
pub struct ToolsResult {
    /// Sorted by the bytes of their results in total, most first.
    pub tools: Vec<ToolUsage>,
}

/// The calls of one tool about one command or path, or about anything when its tool doesn't say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolUsage {
    /// The provider's name of the tool, such as `Bash`.
    pub tool: String,
    /// What the commands start with, the program and its first argument unless that is an option or a path, such as `gh api`.
    pub command: Option<String>,
    /// The file that the calls read or changed, relative to the clone's checkout when it is inside it.
    pub path: Option<String>,
    /// The calls, the denied ones included.
    pub calls: u64,
    /// The average size of their results in tokens, as bytes / 4, over the calls that report one; null when none does.
    pub average_result_tokens: Option<u64>,
    /// The calls that ran and failed.
    pub errors: u64,
    /// The calls whose previous call of the same tool and command or path in their session failed.
    pub retries: u64,
    /// The calls that the clone's permissions denied.
    pub denials: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum ToolsError {
    #[error("no project {name}")]
    UnknownProject { name: String },
}

impl MethodError for ToolsError {
    // usage.* has the codes 1300 to 1399.
    const ERRORS: &'static [ErrorCode] = &[ErrorCode {
        variant: "UnknownProject",
        code: 1300,
        message: "No such project",
    }];
}
