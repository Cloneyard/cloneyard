use std::future;
use std::sync::Arc;

use cloneyard_api::rpc::Discover;
use cloneyard_api::system::{Ping, PingResult, Version, VersionResult};
use cloneyard_api::{account, clone, project};
use cloneyard_rpc::{Error, HandlerError, Router};

use crate::node::Node;

pub(crate) fn router(node: Arc<Node>) -> Router {
    let document = cloneyard_api::document();
    let node_id = node.id;
    Router::new()
        .handle::<Ping>(move |()| future::ready(Ok(PingResult { node_id })))
        .handle::<Version>(|()| async {
            Ok(VersionResult {
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })
        })
        .handle::<project::Add>({
            let node = Arc::clone(&node);
            move |params| {
                let node = Arc::clone(&node);
                async move { reply(crate::project::add(&node, params).await) }
            }
        })
        .handle::<project::List>({
            let node = Arc::clone(&node);
            move |()| {
                let node = Arc::clone(&node);
                async move { reply(crate::project::list(&node).await.map(Ok)) }
            }
        })
        .handle::<account::Add>({
            let node = Arc::clone(&node);
            move |params| {
                let node = Arc::clone(&node);
                async move { reply(crate::account::add(&node, params).await) }
            }
        })
        .handle::<account::List>({
            let node = Arc::clone(&node);
            move |()| {
                let node = Arc::clone(&node);
                async move { reply(crate::account::list(&node).await.map(Ok)) }
            }
        })
        .handle::<clone::Run>({
            let node = Arc::clone(&node);
            move |params| {
                let node = Arc::clone(&node);
                async move { reply(crate::clone::run(&node, params).await) }
            }
        })
        .handle::<clone::List>({
            let node = Arc::clone(&node);
            move |()| {
                let node = Arc::clone(&node);
                async move { reply(crate::clone::list(&node).await.map(Ok)) }
            }
        })
        .handle::<clone::Show>({
            let node = Arc::clone(&node);
            move |params| {
                let node = Arc::clone(&node);
                async move { reply(crate::clone::show(&node, params).await) }
            }
        })
        .handle::<clone::Stop>(move |params| {
            let node = Arc::clone(&node);
            async move { reply(crate::clone::stop(&node, params).await) }
        })
        .handle::<Discover>(move |()| future::ready(Ok(document.clone())))
}

/// Sends a method's own error with its code, and any other failure as -32603.
fn reply<T, E>(result: anyhow::Result<Result<T, E>>) -> Result<T, HandlerError<E>> {
    match result {
        Ok(reply) => reply.map_err(HandlerError::Method),
        Err(error) => Err(HandlerError::Rpc(Error::internal(format!("{error:#}")))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use cloneyard_api::system::Ping;
    use cloneyard_rpc::Client;
    use tempfile::TempDir;
    use tokio::io::{DuplexStream, ReadHalf, WriteHalf, duplex, split};

    use super::router;
    use crate::node::Node;

    pub(crate) type TestClient = Client<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>;

    pub(crate) async fn open_node(dir: &TempDir) -> Arc<Node> {
        Arc::new(Node::open(dir.path()).await.unwrap())
    }

    pub(crate) fn connect(node: Arc<Node>) -> TestClient {
        let router = router(node);
        let (client, server) = duplex(64 * 1024);
        tokio::spawn(async move {
            let (reader, writer) = split(server);
            cloneyard_rpc::serve(&router, reader, writer).await
        });
        let (reader, writer) = split(client);
        Client::new(reader, writer)
    }

    #[tokio::test]
    async fn serves_exactly_the_methods_in_the_document() {
        let dir = TempDir::new().unwrap();
        let mut documented: Vec<String> = cloneyard_api::document()
            .methods
            .into_iter()
            .map(|method| method.name)
            .collect();
        documented.sort_unstable();

        assert_eq!(router(open_node(&dir).await).methods(), documented);
    }

    #[tokio::test]
    async fn answers_ping_with_the_id_of_the_node() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;

        let ping = connect(Arc::clone(&node))
            .request::<Ping>(())
            .await
            .unwrap();

        assert_eq!(ping.node_id, node.id);
    }
}
