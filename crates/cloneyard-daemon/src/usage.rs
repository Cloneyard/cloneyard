use cloneyard_api::usage::{ToolsError, ToolsParams, ToolsResult};

use crate::node::Node;

pub(crate) async fn tools(
    node: &Node,
    params: ToolsParams,
) -> anyhow::Result<Result<ToolsResult, ToolsError>> {
    let ToolsParams { since, project } = params;
    let project_id = match project {
        Some(name) => {
            let Some(project) = node
                .store
                .projects(node.id)
                .await?
                .into_iter()
                .find(|known| known.name == name)
            else {
                return Ok(Err(ToolsError::UnknownProject { name }));
            };
            Some(project.id)
        }
        None => None,
    };
    let tools = node.store.tool_usage(node.id, since, project_id).await?;
    Ok(Ok(ToolsResult { tools }))
}

#[cfg(test)]
mod tests {
    use cloneyard_api::usage::{Tools, ToolsError, ToolsParams, ToolsResult};
    use cloneyard_rpc::ClientError;
    use tempfile::TempDir;

    use crate::router::tests::{connect, open_node};

    #[tokio::test]
    async fn tools_counts_every_project_or_one_and_names_a_project_it_does_not_know() {
        let dir = TempDir::new().unwrap();
        let node = open_node(&dir).await;
        node.store
            .add_project(node.id, "app", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        let mut client = connect(node);
        let params = |project: Option<&str>| ToolsParams {
            since: 0,
            project: project.map(str::to_owned),
        };

        let every = client.request::<Tools>(params(None)).await.unwrap();
        let app = client.request::<Tools>(params(Some("app"))).await.unwrap();
        let web = client.request::<Tools>(params(Some("web"))).await;

        assert_eq!(every, ToolsResult { tools: Vec::new() });
        assert_eq!(app, every);
        match web {
            Err(ClientError::Method(error)) => assert_eq!(
                error,
                ToolsError::UnknownProject {
                    name: "web".to_owned()
                }
            ),
            other => panic!("expected an error of usage.tools, got {other:?}"),
        }
    }
}
