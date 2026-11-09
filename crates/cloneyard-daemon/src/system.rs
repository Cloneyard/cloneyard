use cloneyard_rpc::Router;
use serde::{Deserialize, Serialize};

pub const PING: &str = "system.ping";
pub const VERSION: &str = "system.version";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
}

pub(crate) fn router() -> Router {
    Router::new()
        .method(PING, |(): ()| async { Ok("pong") })
        .method(VERSION, |(): ()| async {
            Ok(Version {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        })
}
