use cloneyard_rpc::Router;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

pub const PING: &str = "system.ping";
pub const VERSION: &str = "system.version";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ping {
    pub node_id: Ulid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
}

pub(crate) fn router(node_id: Ulid) -> Router {
    Router::new()
        .method(PING, move |(): ()| async move { Ok(Ping { node_id }) })
        .method(VERSION, |(): ()| async {
            Ok(Version {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        })
}
