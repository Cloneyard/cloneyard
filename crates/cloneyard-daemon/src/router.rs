use cloneyard_api::system::{Ping, PingResult, Version, VersionResult};
use cloneyard_rpc::Router;
use ulid::Ulid;

pub(crate) fn router(node_id: Ulid) -> Router {
    Router::new()
        .handle::<Ping>(move |()| async move { Ok(PingResult { node_id }) })
        .handle::<Version>(|()| async {
            Ok(VersionResult {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        })
}
