use anyhow::Context as _;
use cloneyard_api::clone::{Clone, Handle, Mode, Session, State};
use rusqlite::{Connection, Params};
use serde_json::{Value, json};
use ulid::Ulid;

use crate::store::{Store, ulid};
use crate::write::Entity;

/// A clone to add, without its handle and its branch, which the node's next handle decides.
#[derive(Debug, Clone)]
pub struct NewClone {
    pub id: Ulid,
    pub project_id: Ulid,
    pub repo_id: Ulid,
    pub account_id: Ulid,
    pub mode: Mode,
    /// The path of its checkout on the node.
    pub checkout: String,
    /// The provider's id of its first session.
    pub session: String,
    /// Whether it runs without the sandbox.
    pub unsandboxed: bool,
}

impl Store {
    /// Adds a `working` clone on node `node_id` with the node's next handle, the branch that `branch` names for that handle, and its first session.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn add_clone(
        &self,
        node_id: Ulid,
        clone: NewClone,
        branch: impl FnOnce(Handle) -> String + Send + 'static,
    ) -> anyhow::Result<Clone> {
        let id = clone.id;
        self.write(move |changes| {
            let number = changes.next_number("clone_handle")?;
            let handle = Handle::new(
                u32::try_from(number).with_context(|| format!("no handle left after {number}"))?,
            );
            changes.insert(
                Entity::Clone,
                &[
                    ("id", json!(clone.id)),
                    ("node_id", json!(node_id)),
                    ("handle", json!(number)),
                    ("project_id", json!(clone.project_id)),
                    ("repo_id", json!(clone.repo_id)),
                    ("account_id", json!(clone.account_id)),
                    ("mode", json!(clone.mode)),
                    ("state", json!(State::Working)),
                    ("reason", Value::Null),
                    ("branch", json!(branch(handle))),
                    ("checkout", json!(clone.checkout)),
                    ("unsandboxed", json!(clone.unsandboxed)),
                ],
            )?;
            changes.insert(
                Entity::Session,
                &[
                    ("id", json!(Ulid::generate())),
                    ("clone_id", json!(clone.id)),
                    ("provider_id", json!(clone.session)),
                ],
            )
        })
        .await
        .context("cannot add a clone")?;
        let clone = self
            .call(move |connection| read(connection, "clone.id = ?1", [id.to_string()]))
            .await
            .with_context(|| format!("cannot read clone {id}"))?;
        clone
            .into_iter()
            .next()
            .with_context(|| format!("no clone {id}"))
    }

    /// Sets the state of clone `id`, with `reason` for `error`.
    ///
    /// # Errors
    ///
    /// When there is no such clone, or the database can't be written.
    pub async fn set_clone_state(
        &self,
        id: Ulid,
        state: State,
        reason: Option<String>,
    ) -> anyhow::Result<()> {
        self.write(move |changes| {
            changes.update(
                Entity::Clone,
                &[
                    ("id", json!(id)),
                    ("state", json!(state)),
                    ("reason", json!(reason)),
                ],
            )
        })
        .await
        .with_context(|| format!("cannot set the state of clone {id}"))
    }

    /// Adds a session with the provider's id `provider_id` to clone `clone_id`.
    ///
    /// # Errors
    ///
    /// When there is no such clone, the session is already known, or the database can't be written.
    pub async fn add_session(&self, clone_id: Ulid, provider_id: &str) -> anyhow::Result<()> {
        let provider_id = provider_id.to_owned();
        self.write(move |changes| {
            changes.insert(
                Entity::Session,
                &[
                    ("id", json!(Ulid::generate())),
                    ("clone_id", json!(clone_id)),
                    ("provider_id", json!(provider_id)),
                ],
            )
        })
        .await
        .with_context(|| format!("cannot add a session to clone {clone_id}"))
    }

    /// The clones of node `node_id`, sorted by handle.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds an invalid value.
    pub async fn clones(&self, node_id: Ulid) -> anyhow::Result<Vec<Clone>> {
        self.call(move |connection| read(connection, "clone.node_id = ?1", [node_id.to_string()]))
            .await
            .context("cannot list the clones")
    }

    /// The clone of node `node_id` with `handle`, or `None` when it has none.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds an invalid value.
    pub async fn clone_by_handle(
        &self,
        node_id: Ulid,
        handle: Handle,
    ) -> anyhow::Result<Option<Clone>> {
        self.call(move |connection| {
            read(
                connection,
                "clone.node_id = ?1 AND clone.handle = ?2",
                (node_id.to_string(), handle.number()),
            )
        })
        .await
        .map(|clones| clones.into_iter().next())
        .with_context(|| format!("cannot read clone {handle}"))
    }

    /// Sets every `working` clone of node `node_id` to `error` with `reason`, and gives their handles.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn lose_clones(&self, node_id: Ulid, reason: &str) -> anyhow::Result<Vec<Handle>> {
        let reason = reason.to_owned();
        self.write(move |changes| {
            let working: Vec<(String, u32)> = changes.query_rows(
                "SELECT id, handle FROM clone WHERE node_id = ?1 AND state = ?2 ORDER BY handle",
                (node_id.to_string(), State::Working.to_string()),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            for (id, _) in &working {
                changes.update(
                    Entity::Clone,
                    &[
                        ("id", json!(id)),
                        ("state", json!(State::Error)),
                        ("reason", json!(reason)),
                    ],
                )?;
            }
            Ok(working
                .into_iter()
                .map(|(_, number)| Handle::new(number))
                .collect())
        })
        .await
        .context("cannot mark the lost clones")
    }
}

fn read(
    connection: &Connection,
    condition: &str,
    params: impl Params,
) -> anyhow::Result<Vec<Clone>> {
    let mut statement = connection.prepare(&format!(
        "SELECT clone.id, clone.handle, project.name, account.name, clone.mode, clone.state, \
         clone.reason, clone.branch, clone.checkout, clone.unsandboxed FROM clone \
         JOIN project ON project.id = clone.project_id \
         JOIN account ON account.id = clone.account_id \
         WHERE {condition} ORDER BY clone.handle"
    ))?;
    let rows = statement.query_map(params, |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, bool>(9)?,
        ))
    })?;
    let mut sessions = connection
        .prepare("SELECT id, provider_id FROM session WHERE clone_id = ?1 ORDER BY rowid")?;
    rows.map(|row| {
        let (id, handle, project, account, mode, state, reason, branch, checkout, unsandboxed) =
            row?;
        let sessions = sessions
            .query_map([&id], |row| Ok((row.get::<_, String>(0)?, row.get(1)?)))?
            .map(|session| {
                let (id, provider_id) = session?;
                Ok(Session {
                    id: ulid(&id)?,
                    provider_id,
                })
            })
            .collect::<anyhow::Result<_>>()?;
        Ok(Clone {
            id: ulid(&id)?,
            handle: Handle::new(handle),
            project,
            account,
            mode: mode_of(&mode)?,
            state: state_of(&state)?,
            reason,
            branch,
            checkout,
            unsandboxed,
            sessions,
        })
    })
    .collect()
}

fn mode_of(text: &str) -> anyhow::Result<Mode> {
    serde_json::from_value(Value::String(text.to_owned()))
        .with_context(|| format!("invalid mode {text:?}"))
}

fn state_of(text: &str) -> anyhow::Result<State> {
    serde_json::from_value(Value::String(text.to_owned()))
        .with_context(|| format!("invalid state {text:?}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicI64;

    use cloneyard_api::account::Provider;
    use cloneyard_api::clone::{Clone, Handle, Mode, Session, State};
    use serde_json::json;
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::NewClone;
    use crate::Store;
    use crate::store::tests::{Logged, NOW, change_log, open, open_at, sql};

    struct Fixture {
        node: Ulid,
        project: Ulid,
        repo: Ulid,
        account: Ulid,
    }

    async fn fixture(store: &Store) -> Fixture {
        let node_id = store.node_id().await.unwrap();
        let project = store
            .add_project(node_id, "app", "app", "/code/app")
            .await
            .unwrap()
            .unwrap();
        let account = store
            .add_account(node_id, "personal", Provider::ClaudeCode, "/data/personal")
            .await
            .unwrap()
            .unwrap();
        Fixture {
            node: node_id,
            project: project.id,
            repo: project.repos[0].id,
            account: account.id,
        }
    }

    async fn add(store: &Store, fixture: &Fixture, session: &str) -> Clone {
        let id = Ulid::generate();
        let clone = NewClone {
            id,
            project_id: fixture.project,
            repo_id: fixture.repo,
            account_id: fixture.account,
            mode: Mode::Headless,
            checkout: format!("/data/clones/{id}/checkout"),
            session: session.to_owned(),
            unsandboxed: false,
        };
        store
            .add_clone(fixture.node, clone, |handle| format!("branch-{handle}"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn adds_a_working_clone_with_its_handle_branch_and_first_session_and_keeps_it_after_reopening()
     {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;

        let clone = add(&store, &fixture, "2f6c").await;
        drop(store);
        let listed = open(&dir).await.clones(fixture.node).await.unwrap();

        let [Session { provider_id, .. }] = clone.sessions.as_slice() else {
            panic!("{clone:?}");
        };
        assert_eq!(
            (
                clone.handle,
                clone.project.as_str(),
                clone.account.as_str(),
                clone.mode,
                clone.state,
                clone.reason.as_deref(),
                clone.branch.as_str(),
                provider_id.as_str()
            ),
            (
                Handle::new(1),
                "app",
                "personal",
                Mode::Headless,
                State::Working,
                None,
                "branch-CT-0001",
                "2f6c"
            )
        );
        assert_eq!(
            clone.checkout,
            format!("/data/clones/{}/checkout", clone.id)
        );
        assert_eq!(listed, [clone]);
    }

    #[tokio::test]
    async fn a_clone_added_unsandboxed_reads_back_unsandboxed() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let id = Ulid::generate();
        let clone = NewClone {
            id,
            project_id: fixture.project,
            repo_id: fixture.repo,
            account_id: fixture.account,
            mode: Mode::Headless,
            checkout: format!("/data/clones/{id}/checkout"),
            session: "2f6c".to_owned(),
            unsandboxed: true,
        };

        let added = store
            .add_clone(fixture.node, clone, |handle| format!("branch-{handle}"))
            .await
            .unwrap();

        assert!(added.unsandboxed);
        assert_eq!(store.clones(fixture.node).await.unwrap(), [added]);
    }

    #[tokio::test]
    async fn hands_out_the_next_handle_also_after_reopening_and_lists_only_the_clones_of_the_node()
    {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        add(&store, &fixture, "a").await;
        add(&store, &fixture, "b").await;
        drop(store);
        let store = open(&dir).await;

        let third = add(&store, &fixture, "c").await;

        let handles: Vec<Handle> = store
            .clones(fixture.node)
            .await
            .unwrap()
            .into_iter()
            .map(|clone| clone.handle)
            .collect();
        assert_eq!(third.handle, Handle::new(3));
        assert_eq!(handles, [1, 2, 3].map(Handle::new));
        assert_eq!(store.clones(Ulid::generate()).await.unwrap(), []);
    }

    #[tokio::test]
    async fn logs_a_clone_and_its_first_session_as_upserts() {
        let dir = TempDir::new().unwrap();
        let now = Arc::new(AtomicI64::new(NOW));
        let store = open_at(&dir, &now).await;
        let fixture = fixture(&store).await;

        let clone = add(&store, &fixture, "2f6c").await;

        let logged: Vec<Logged> = change_log(&dir).into_iter().skip(5).collect();
        let id = clone.id.to_string();
        let session = clone.sessions[0].id.to_string();
        let node = fixture.node.to_string();
        assert_eq!(
            logged,
            [
                Logged {
                    node_id: node.clone(),
                    hlc: (NOW << 16) + 5,
                    entity: "clone".to_owned(),
                    entity_id: id.clone(),
                    op: "upsert".to_owned(),
                    fields: json!({
                        "id": id,
                        "node_id": node,
                        "handle": 1,
                        "project_id": fixture.project,
                        "repo_id": fixture.repo,
                        "account_id": fixture.account,
                        "mode": "headless",
                        "state": "working",
                        "reason": null,
                        "branch": "branch-CT-0001",
                        "checkout": clone.checkout,
                        "unsandboxed": false,
                    }),
                },
                Logged {
                    node_id: node,
                    hlc: (NOW << 16) + 6,
                    entity: "session".to_owned(),
                    entity_id: session.clone(),
                    op: "upsert".to_owned(),
                    fields: json!({"id": session, "clone_id": id, "provider_id": "2f6c"}),
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_new_state_logs_only_the_id_and_the_fields_it_set() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "2f6c").await;

        store
            .set_clone_state(clone.id, State::Error, Some("api_error: boom".to_owned()))
            .await
            .unwrap();

        let last = change_log(&dir).pop().unwrap();
        let shown = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                last.entity.as_str(),
                last.entity_id,
                last.op.as_str(),
                last.fields
            ),
            (
                "clone",
                clone.id.to_string(),
                "upsert",
                json!({"id": clone.id, "state": "error", "reason": "api_error: boom"})
            )
        );
        assert_eq!(
            (shown.state, shown.reason.as_deref()),
            (State::Error, Some("api_error: boom"))
        );
    }

    #[tokio::test]
    async fn names_the_clone_when_it_sets_the_state_of_one_that_does_not_exist_and_writes_nothing()
    {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let id = Ulid::generate();

        let error = store
            .set_clone_state(id, State::Done, None)
            .await
            .unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            format!("cannot set the state of clone {id}: no clone {id}")
        );
        assert_eq!(change_log(&dir), []);
    }

    #[tokio::test]
    async fn lists_the_sessions_of_a_clone_in_the_order_they_started() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;

        for session in ["second", "third"] {
            store.add_session(clone.id, session).await.unwrap();
        }

        let shown = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap();
        let sessions: Vec<&str> = shown
            .sessions
            .iter()
            .map(|session| session.provider_id.as_str())
            .collect();
        assert_eq!(sessions, ["first", "second", "third"]);
    }

    #[tokio::test]
    async fn finds_no_clone_for_a_handle_the_node_never_gave_out() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        add(&store, &fixture, "2f6c").await;

        let found = store
            .clone_by_handle(fixture.node, Handle::new(2))
            .await
            .unwrap();

        assert_eq!(found, None);
    }

    #[tokio::test]
    async fn loses_only_the_working_clones_of_the_node() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        let done = add(&store, &fixture, "a").await;
        let working = add(&store, &fixture, "b").await;
        add(&store, &fixture, "c").await;
        store
            .set_clone_state(done.id, State::Done, None)
            .await
            .unwrap();

        let lost = store
            .lose_clones(fixture.node, "lost while the daemon was down")
            .await
            .unwrap();
        let other_node = store.lose_clones(Ulid::generate(), "lost").await.unwrap();

        let states: Vec<(State, Option<String>)> = store
            .clones(fixture.node)
            .await
            .unwrap()
            .into_iter()
            .map(|clone| (clone.state, clone.reason))
            .collect();
        let lost_reason = Some("lost while the daemon was down".to_owned());
        assert_eq!(lost, [working.handle, Handle::new(3)]);
        assert_eq!(other_node, []);
        assert_eq!(
            states,
            [
                (State::Done, None),
                (State::Error, lost_reason.clone()),
                (State::Error, lost_reason)
            ]
        );
    }

    #[tokio::test]
    async fn names_the_state_when_the_database_holds_an_unknown_one() {
        let dir = TempDir::new().unwrap();
        let store = open(&dir).await;
        let fixture = fixture(&store).await;
        add(&store, &fixture, "2f6c").await;
        sql(&dir)
            .execute("UPDATE clone SET state = 'idle'", [])
            .unwrap();

        let error = store.clones(fixture.node).await.unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "cannot list the clones: invalid state \"idle\": \
             unknown variant `idle`, expected one of `working`, `done`, `error`, `stopped`"
        );
    }
}
