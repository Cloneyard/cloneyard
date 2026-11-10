use std::future;

use cloneyard_api::rpc::Discover;
use cloneyard_api::system::{Ping, PingResult, Version, VersionResult};
use cloneyard_rpc::Router;
use ulid::Ulid;

pub(crate) fn router(node_id: Ulid) -> Router {
    let document = cloneyard_api::document();
    Router::new()
        .handle::<Ping>(move |()| async move { Ok(PingResult { node_id }) })
        .handle::<Version>(|()| async {
            Ok(VersionResult {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        })
        .handle::<Discover>(move |()| future::ready(Ok(document.clone())))
}

#[cfg(test)]
mod tests {
    use ulid::Ulid;

    use super::router;

    #[test]
    fn serves_exactly_the_methods_in_the_document() {
        let mut documented: Vec<String> = cloneyard_api::document()
            .methods
            .into_iter()
            .map(|method| method.name)
            .collect();
        documented.sort_unstable();

        assert_eq!(router(Ulid::nil()).methods(), documented);
    }
}
