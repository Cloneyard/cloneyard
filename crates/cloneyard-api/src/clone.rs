use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

use crate::{ErrorCode, Method, MethodError, NoError};

pub struct Run;

impl Method for Run {
    const NAME: &'static str = "clone.run";
    const SUMMARY: Option<&'static str> =
        Some("Starts a headless clone on an account, in a checkout of a project's repo");
    type Params = RunParams;
    type Result = Clone;
    type Error = RunError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunParams {
    /// The name of the project whose repo the clone gets a checkout of.
    pub project: String,
    /// The name of the account the clone runs on.
    pub account: String,
    /// The first message to the clone.
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum RunError {
    #[error("no project {name}")]
    UnknownProject { name: String },
    #[error("no account {name}")]
    UnknownAccount { name: String },
    /// The kernel has no Landlock, and the node's settings in `config` don't say `unsandboxed = true`.
    #[error(
        "this kernel has no Landlock, so clones can't run in the sandbox; \
         set unsandboxed = true in {config} to run them without it"
    )]
    NoSandbox { config: String },
}

impl MethodError for RunError {
    // clone.* has the codes 1200 to 1299.
    const ERRORS: &'static [ErrorCode] = &[UNKNOWN_PROJECT, UNKNOWN_ACCOUNT, NO_SANDBOX];
}

const UNKNOWN_PROJECT: ErrorCode = ErrorCode {
    variant: "UnknownProject",
    code: 1200,
    message: "No such project",
};

const UNKNOWN_ACCOUNT: ErrorCode = ErrorCode {
    variant: "UnknownAccount",
    code: 1201,
    message: "No such account",
};

const UNKNOWN_CLONE: ErrorCode = ErrorCode {
    variant: "UnknownClone",
    code: 1202,
    message: "No such clone",
};

const NOT_RUNNING: ErrorCode = ErrorCode {
    variant: "NotRunning",
    code: 1203,
    message: "Clone is not running",
};

const NO_SANDBOX: ErrorCode = ErrorCode {
    variant: "NoSandbox",
    code: 1204,
    message: "No sandbox for clones on this node",
};

pub struct List;

impl Method for List {
    const NAME: &'static str = "clone.list";
    const SUMMARY: Option<&'static str> = Some("Returns the clones on this node");
    type Params = ();
    type Result = ListResult;
    type Error = NoError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "CloneListResult")]
pub struct ListResult {
    /// Sorted by handle.
    pub clones: Vec<Clone>,
}

pub struct Show;

impl Method for Show {
    const NAME: &'static str = "clone.show";
    const SUMMARY: Option<&'static str> = Some("Returns a clone by its handle");
    type Params = HandleParams;
    type Result = Clone;
    type Error = ShowError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandleParams {
    pub handle: Handle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum ShowError {
    #[error("no clone {handle}")]
    UnknownClone { handle: Handle },
}

impl MethodError for ShowError {
    const ERRORS: &'static [ErrorCode] = &[UNKNOWN_CLONE];
}

pub struct Stop;

impl Method for Stop {
    const NAME: &'static str = "clone.stop";
    const SUMMARY: Option<&'static str> =
        Some("Ends a clone's process and returns the clone once it has ended");
    type Params = HandleParams;
    type Result = Clone;
    type Error = StopError;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
pub enum StopError {
    #[error("no clone {handle}")]
    UnknownClone { handle: Handle },
    #[error("clone {handle} is {state}, not running")]
    NotRunning { handle: Handle, state: State },
}

impl MethodError for StopError {
    const ERRORS: &'static [ErrorCode] = &[UNKNOWN_CLONE, NOT_RUNNING];
}

/// A coding agent that Cloneyard runs on an account, in a checkout of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Clone {
    #[schemars(schema_with = "crate::ulid_schema")]
    pub id: Ulid,
    pub handle: Handle,
    /// The name of the project whose repo the checkout is of.
    pub project: String,
    /// The name of the account the clone runs on.
    pub account: String,
    pub mode: Mode,
    pub state: State,
    /// Why the clone ended in `error`, or null in any other state.
    pub reason: Option<String>,
    /// The branch of the checkout, which is the clone's own.
    pub branch: String,
    /// The path of the checkout on this node.
    pub checkout: String,
    /// Whether the clone runs without the sandbox: when the node's settings say `unsandboxed = true`, and for every clone started before the sandbox existed.
    pub unsandboxed: bool,
    /// In the order they started.
    pub sessions: Vec<Session>,
}

/// One provider session of a clone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    #[schemars(schema_with = "crate::ulid_schema")]
    pub id: Ulid,
    /// The provider's id of the session, such as Claude Code's `session_id`.
    pub provider_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Runs without a terminal, from a prompt, and reports its state through its output.
    Headless,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Headless => f.pad("headless"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// It works on its turn.
    Working,
    /// It waits for a permission.
    Blocked,
    /// It ended its turn and waits for a message.
    Idle,
    /// It ended its turn without an error.
    Done,
    /// It failed, or its process ended or was lost before its turn ended.
    Error,
    /// It was stopped on request.
    Stopped,
}

impl State {
    /// The states of a clone whose process runs.
    pub const RUNNING: [Self; 3] = [Self::Working, Self::Blocked, Self::Idle];

    /// Whether the clone's process runs, as it does while it is working, blocked or idle.
    #[must_use]
    pub fn runs(self) -> bool {
        Self::RUNNING.contains(&self)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Error => "error",
            Self::Stopped => "stopped",
        })
    }
}

/// A clone's short name on its node, such as `CT-0042`; another node can give out the same one, so only the id identifies a clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle(u32);

impl Handle {
    #[must_use]
    pub fn new(number: u32) -> Self {
        Self(number)
    }

    #[must_use]
    pub fn number(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&format!("CT-{:04}", self.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid handle {0:?}: a handle is CT- and a number, such as CT-0042")]
pub struct InvalidHandle(String);

/// Reads `CT-0042`, and also `ct-42`, as people type it.
impl FromStr for Handle {
    type Err = InvalidHandle;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.get(..3)
            .filter(|prefix| prefix.eq_ignore_ascii_case("CT-"))
            .and_then(|_| text.get(3..))
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse().ok())
            .map(Self)
            .ok_or_else(|| InvalidHandle(text.to_owned()))
    }
}

impl Serialize for Handle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Handle {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Handle {
    fn schema_name() -> Cow<'static, str> {
        "Handle".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A clone's short name on its node, such as `CT-0042`.",
            "type": "string",
            "pattern": "^CT-[0-9]{4,}$",
        })
    }
}
