use anyhow::Context as _;
use cloneyard_api::clone::{HookEvent, State};
use rusqlite::OptionalExtension as _;
use serde_json::json;
use ulid::Ulid;

use crate::clone::{session, state_of};
use crate::store::{Store, ulid};
use crate::write::Entity;

/// A hook event of a clone to add, as its spool holds it.
#[derive(Debug, Clone)]
pub struct NewHookEvent {
    pub clone_id: Ulid,
    /// The provider's id of the session the hook ran in.
    pub session: String,
    pub hook: String,
    pub tool: Option<String>,
    /// When the hook ran, in milliseconds since the Unix epoch.
    pub at: i64,
    /// The name of its file in the spool.
    pub file: String,
    pub payload: String,
    /// The state that the hook gives a running clone.
    pub state: Option<State>,
}

/// Why a hook event was not added.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("no clone {0}")]
    UnknownClone(Ulid),
    #[error("session {session} belongs to clone {owner}")]
    ForeignSession { session: String, owner: Ulid },
}

impl Store {
    /// Adds a hook event of a clone in one transaction, with its session when that is new, and gives a running clone the event's state; a file the clone's events already have adds nothing.
    ///
    /// # Errors
    ///
    /// When the database can't be read or written.
    pub async fn add_hook_event(&self, event: NewHookEvent) -> anyhow::Result<Result<(), Refused>> {
        let NewHookEvent {
            clone_id,
            session: provider_id,
            hook,
            tool,
            at,
            file,
            payload,
            state,
        } = event;
        self.write(move |changes| {
            let current: Option<String> = changes
                .query_row(
                    "SELECT state FROM clone WHERE id = ?1",
                    [clone_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(Err(Refused::UnknownClone(clone_id)));
            };
            let taken: bool = changes.query_row(
                "SELECT EXISTS (SELECT 1 FROM hook_event WHERE clone_id = ?1 AND file = ?2)",
                (clone_id.to_string(), &file),
                |row| row.get(0),
            )?;
            if taken {
                return Ok(Ok(()));
            }
            let session_id = match session(changes, clone_id, &provider_id)? {
                Ok(id) => id,
                Err(owner) => {
                    return Ok(Err(Refused::ForeignSession {
                        session: provider_id,
                        owner,
                    }));
                }
            };
            changes.append(
                "hook_event",
                &[
                    ("id", json!(Ulid::generate())),
                    ("clone_id", json!(clone_id)),
                    ("session_id", json!(session_id)),
                    ("hook", json!(hook)),
                    ("tool", json!(tool)),
                    ("at", json!(at)),
                    ("file", json!(file)),
                    ("payload", json!(payload)),
                ],
            )?;
            let current = state_of(&current)?;
            if let Some(state) = state
                && current.runs()
                && state != current
            {
                changes.update(
                    Entity::Clone,
                    &[("id", json!(clone_id)), ("state", json!(state))],
                )?;
            }
            Ok(Ok(()))
        })
        .await
        .with_context(|| format!("cannot add a hook event of clone {clone_id}"))
    }

    /// The last `limit` hook events of clone `clone_id`, oldest first.
    ///
    /// # Errors
    ///
    /// When the database can't be read or holds an invalid id.
    pub async fn hook_events(&self, clone_id: Ulid, limit: u32) -> anyhow::Result<Vec<HookEvent>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT hook_event.id, hook_event.at, session.provider_id, hook_event.hook, \
                 hook_event.tool FROM hook_event \
                 JOIN session ON session.id = hook_event.session_id \
                 WHERE hook_event.clone_id = ?1 ORDER BY hook_event.seq DESC LIMIT ?2",
            )?;
            let rows = statement.query_map((clone_id.to_string(), limit), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?;
            let mut events = rows
                .map(|row| {
                    let (id, at, session, hook, tool) = row?;
                    Ok(HookEvent {
                        id: ulid(&id)?,
                        at,
                        session,
                        hook,
                        tool,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            events.reverse();
            Ok(events)
        })
        .await
        .with_context(|| format!("cannot read the hook events of clone {clone_id}"))
    }
}

#[cfg(test)]
mod tests {
    use cloneyard_api::clone::{Clone, HookEvent, State};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use ulid::Ulid;

    use super::{NewHookEvent, Refused};
    use crate::Store;
    use crate::clone::tests::{Fixture, add, fixture};
    use crate::store::tests::{change_log, open, sql};

    fn event(clone: &Clone, session: &str, hook: &str, file: &str) -> NewHookEvent {
        NewHookEvent {
            clone_id: clone.id,
            session: session.to_owned(),
            hook: hook.to_owned(),
            tool: None,
            at: 1_764_000_000_000,
            file: file.to_owned(),
            payload: json!({"session_id": session}).to_string(),
            state: None,
        }
    }

    async fn with_clone(dir: &TempDir) -> (Store, Fixture, Clone) {
        let store = open(dir).await;
        let fixture = fixture(&store).await;
        let clone = add(&store, &fixture, "first").await;
        (store, fixture, clone)
    }

    fn hooks(events: &[HookEvent]) -> Vec<(&str, &str)> {
        events
            .iter()
            .map(|event| (event.hook.as_str(), event.session.as_str()))
            .collect()
    }

    #[tokio::test]
    async fn adds_events_with_their_session_and_tool_and_reads_the_last_ones_oldest_first() {
        let dir = TempDir::new().unwrap();
        let (store, fixture, clone) = with_clone(&dir).await;
        for (session, hook, file) in [
            ("first", "SessionStart", "1"),
            ("first", "PreToolUse", "2"),
            ("second", "SessionStart", "3"),
            ("second", "Stop", "4"),
        ] {
            let mut event = event(&clone, session, hook, file);
            event.tool = (hook == "PreToolUse").then(|| "Bash".to_owned());
            store.add_hook_event(event).await.unwrap().unwrap();
        }

        let last = store.hook_events(clone.id, 3).await.unwrap();
        let all = store.hook_events(clone.id, 10).await.unwrap();

        let sessions: Vec<String> = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap()
            .sessions
            .into_iter()
            .map(|session| session.provider_id)
            .collect();
        assert_eq!(
            hooks(&last),
            [
                ("PreToolUse", "first"),
                ("SessionStart", "second"),
                ("Stop", "second")
            ]
        );
        assert_eq!(all.len(), 4);
        assert_eq!(
            (all[1].tool.as_deref(), all[1].at),
            (Some("Bash"), 1_764_000_000_000)
        );
        assert_eq!(sessions, ["first", "second"]);
        let payload: String = sql(&dir)
            .query_row(
                "SELECT payload FROM hook_event WHERE file = '4'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap(),
            json!({"session_id": "second"})
        );
    }

    #[tokio::test]
    async fn the_same_file_twice_adds_one_event() {
        let dir = TempDir::new().unwrap();
        let (store, _, clone) = with_clone(&dir).await;

        for _ in 0..2 {
            store
                .add_hook_event(event(&clone, "first", "Stop", "1"))
                .await
                .unwrap()
                .unwrap();
        }

        assert_eq!(store.hook_events(clone.id, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refuses_an_event_of_an_unknown_clone_or_with_another_clones_session_and_writes_nothing()
     {
        let dir = TempDir::new().unwrap();
        let (store, fixture, clone) = with_clone(&dir).await;
        let other = add(&store, &fixture, "other").await;
        let unknown = Ulid::generate();
        let logged = change_log(&dir).len();

        let foreign = store
            .add_hook_event(event(&clone, "other", "Stop", "1"))
            .await
            .unwrap();
        let mut lost = event(&clone, "first", "Stop", "2");
        lost.clone_id = unknown;
        let lost = store.add_hook_event(lost).await.unwrap();

        assert_eq!(
            foreign,
            Err(Refused::ForeignSession {
                session: "other".to_owned(),
                owner: other.id
            })
        );
        assert_eq!(lost, Err(Refused::UnknownClone(unknown)));
        assert_eq!(
            foreign.unwrap_err().to_string(),
            format!("session other belongs to clone {}", other.id)
        );
        assert_eq!(store.hook_events(clone.id, 10).await.unwrap(), []);
        assert_eq!(change_log(&dir).len(), logged);
    }

    #[tokio::test]
    async fn a_running_clone_takes_the_state_of_its_hooks_and_a_done_one_keeps_its_own() {
        let dir = TempDir::new().unwrap();
        let (store, fixture, clone) = with_clone(&dir).await;
        let mut states = Vec::new();
        for (file, state) in [
            ("1", Some(State::Blocked)),
            ("2", Some(State::Working)),
            ("3", None),
            ("4", Some(State::Idle)),
        ] {
            let mut event = event(&clone, "first", "Notification", file);
            event.state = state;
            store.add_hook_event(event).await.unwrap().unwrap();
            let shown = store
                .clone_by_handle(fixture.node, clone.handle)
                .await
                .unwrap()
                .unwrap();
            states.push(shown.state);
        }
        store
            .set_clone_state(clone.id, State::Done, None)
            .await
            .unwrap();
        let mut late = event(&clone, "first", "Stop", "5");
        late.state = Some(State::Idle);

        store.add_hook_event(late).await.unwrap().unwrap();

        let shown = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            states,
            [State::Blocked, State::Working, State::Working, State::Idle]
        );
        assert_eq!(shown.state, State::Done);
    }

    #[tokio::test]
    async fn the_change_log_takes_the_new_session_and_state_but_not_the_event() {
        let dir = TempDir::new().unwrap();
        let (store, _, clone) = with_clone(&dir).await;
        let before = change_log(&dir).len();
        let mut stop = event(&clone, "second", "Stop", "1");
        stop.state = Some(State::Idle);

        store.add_hook_event(stop).await.unwrap().unwrap();

        let logged: Vec<(String, Value)> = change_log(&dir)
            .into_iter()
            .skip(before)
            .map(|logged| (logged.entity, logged.fields))
            .collect();
        assert_eq!(logged.len(), 2);
        assert_eq!(logged[0].0, "session");
        assert_eq!(logged[0].1["provider_id"], json!("second"));
        assert_eq!(
            logged[1],
            ("clone".to_owned(), json!({"id": clone.id, "state": "idle"}))
        );
    }

    #[tokio::test]
    async fn adding_a_session_the_clone_has_adds_nothing_and_one_of_another_clone_is_an_error() {
        let dir = TempDir::new().unwrap();
        let (store, fixture, clone) = with_clone(&dir).await;
        let other = add(&store, &fixture, "other").await;

        store.add_session(clone.id, "first").await.unwrap();
        let error = store.add_session(clone.id, "other").await.unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            format!(
                "cannot add a session to clone {}: session other belongs to clone {}",
                clone.id, other.id
            )
        );
        let shown = store
            .clone_by_handle(fixture.node, clone.handle)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(shown.sessions.len(), 1);
    }
}
