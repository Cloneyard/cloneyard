use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use rusqlite::types::Value as Column;
use rusqlite::{Connection, OptionalExtension as _, Params, Row, Transaction, params_from_iter};
use serde_json::{Map, Value};
use ulid::Ulid;

/// The milliseconds since the Unix epoch, as the wall clock has them.
pub(crate) type Clock = Box<dyn Fn() -> i64 + Send>;

pub(crate) fn system_clock() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// A table whose rows are entities, each change of which `change_log` records.
#[derive(Clone, Copy)]
pub(crate) enum Entity {
    Project,
    Repo,
    RepoPath,
}

impl Entity {
    fn table(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Repo => "repo",
            Self::RepoPath => "repo_path",
        }
    }

    /// The columns whose values, joined with `:`, are the entity's id in `change_log`.
    fn key(self) -> &'static [&'static str] {
        match self {
            Self::Project | Self::Repo => &["id"],
            Self::RepoPath => &["repo_id", "node_id"],
        }
    }
}

/// The node's id, created the first time it is asked for.
pub(crate) fn node_id(connection: &Connection) -> anyhow::Result<Ulid> {
    connection.execute(
        "INSERT OR IGNORE INTO meta (key, value) VALUES ('node_id', ?1)",
        [Ulid::generate().to_string()],
    )?;
    let id: String =
        connection.query_row("SELECT value FROM meta WHERE key = 'node_id'", [], |row| {
            row.get(0)
        })?;
    Ulid::from_string(&id).with_context(|| format!("invalid node id {id:?}"))
}

/// Runs `work` in one transaction, which commits only when `work` succeeds.
pub(crate) fn write<T>(
    connection: &mut Connection,
    clock: &Clock,
    work: impl FnOnce(&mut Changes<'_>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let transaction = connection.transaction()?;
    let node_id = node_id(&transaction)?.to_string();
    let stored: Option<String> = transaction
        .query_row("SELECT value FROM meta WHERE key = 'hlc'", [], |row| {
            row.get(0)
        })
        .optional()?;
    let last = stored
        .map(|hlc| hlc.parse().with_context(|| format!("invalid hlc {hlc:?}")))
        .transpose()?
        .unwrap_or(0);
    let mut changes = Changes {
        transaction,
        node_id,
        wall: clock(),
        hlc: last,
    };
    let done = work(&mut changes)?;
    let Changes {
        transaction, hlc, ..
    } = changes;
    if hlc != last {
        transaction.execute(
            "INSERT INTO meta (key, value) VALUES ('hlc', ?1) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [hlc.to_string()],
        )?;
    }
    transaction.commit()?;
    Ok(done)
}

/// The changes of one write, each made together with its `change_log` row.
pub(crate) struct Changes<'a> {
    transaction: Transaction<'a>,
    node_id: String,
    wall: i64,
    hlc: i64,
}

impl Changes<'_> {
    pub(crate) fn query_row<T, P: Params>(
        &self,
        sql: &str,
        params: P,
        row: impl FnOnce(&Row<'_>) -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        self.transaction.query_row(sql, params, row)
    }

    /// Adds a row to the table of `entity`, with `fields` as its columns.
    pub(crate) fn insert(
        &mut self,
        entity: Entity,
        fields: &[(&'static str, Value)],
    ) -> anyhow::Result<()> {
        let columns: Vec<&str> = fields.iter().map(|(column, _)| *column).collect();
        self.transaction.execute(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                entity.table(),
                columns.join(", "),
                vec!["?"; fields.len()].join(", ")
            ),
            params_from_iter(fields.iter().map(|(_, value)| column(value))),
        )?;
        self.log(entity, "upsert", fields)
    }

    fn log(
        &mut self,
        entity: Entity,
        op: &str,
        fields: &[(&'static str, Value)],
    ) -> anyhow::Result<()> {
        let key = entity
            .key()
            .iter()
            .map(|key| {
                fields
                    .iter()
                    .find(|(column, _)| column == key)
                    .and_then(|(_, value)| value.as_str())
                    .with_context(|| format!("{} needs a text {key}", entity.table()))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.hlc = (self.wall << 16).max(self.hlc + 1);
        let fields: Map<String, Value> = fields
            .iter()
            .map(|(column, value)| ((*column).to_owned(), value.clone()))
            .collect();
        self.transaction.execute(
            "INSERT INTO change_log (id, node_id, hlc, entity, entity_id, op, fields) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                Ulid::generate().to_string(),
                &self.node_id,
                self.hlc,
                entity.table(),
                key.join(":"),
                op,
                Value::Object(fields).to_string(),
            ),
        )?;
        Ok(())
    }
}

// Unlike rusqlite's own conversion, a string goes in as text and not as a JSON string.
fn column(value: &Value) -> Column {
    match value {
        Value::Null => Column::Null,
        Value::Bool(value) => Column::Integer(i64::from(*value)),
        Value::Number(number) => number.as_i64().map_or_else(
            || Column::Real(number.as_f64().unwrap_or(f64::NAN)),
            Column::Integer,
        ),
        Value::String(text) => Column::Text(text.clone()),
        Value::Array(_) | Value::Object(_) => Column::Text(value.to_string()),
    }
}
