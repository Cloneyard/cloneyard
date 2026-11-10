use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::Method;

pub struct Ping;

impl Method for Ping {
    const NAME: &'static str = "system.ping";
    const SUMMARY: &'static str = "Returns the node id, to check that the daemon answers";
    type Params = ();
    type Result = PingResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PingResult {
    /// The id of the node the daemon runs on.
    #[schemars(schema_with = "crate::ulid_schema")]
    pub node_id: Ulid,
}

pub struct Version;

impl Method for Version {
    const NAME: &'static str = "system.version";
    const SUMMARY: &'static str = "Returns the version of the daemon";
    type Params = ();
    type Result = VersionResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VersionResult {
    /// The version of the daemon, such as `0.0.1`.
    pub version: String,
}
