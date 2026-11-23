use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail};
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations, SchemaVersion};

pub(crate) fn migrations() -> Vec<M<'static>> {
    vec![
        M::up(include_str!("../migrations/0001-initial.sql")),
        M::up(include_str!("../migrations/0002-repo-path.sql")),
        M::up(include_str!("../migrations/0003-change-log-ops.sql")),
        M::up(include_str!("../migrations/0004-account.sql")),
    ]
}

pub(crate) fn open(path: &Path, migrations: Vec<M<'static>>) -> anyhow::Result<Connection> {
    let mut connection =
        connect(path).with_context(|| format!("cannot open {}", path.display()))?;
    let latest = migrations.len();
    let migrations = Migrations::new(migrations);
    match migrations
        .current_version(&connection)
        .with_context(|| format!("cannot read the schema version of {}", path.display()))?
    {
        SchemaVersion::Outside(version) => bail!(
            "database schema v{version} is newer than this cloneyard (v{latest}); \
             upgrade cloneyard or restore a file from {}",
            backups_dir(path).display()
        ),
        SchemaVersion::Inside(version) if version.get() < latest => {
            back_up(&connection, path, version.get())?;
        }
        SchemaVersion::Inside(_) | SchemaVersion::NoneSet => {}
    }
    migrations
        .to_latest(&mut connection)
        .with_context(|| format!("cannot migrate {}", path.display()))?;
    Ok(connection)
}

// A file that is not a database fails at the first pragma, not at `Connection::open`.
fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    // With WAL a power loss can drop the last commits, but never corrupts the file.
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

fn back_up(connection: &Connection, path: &Path, version: usize) -> anyhow::Result<()> {
    let dir = backups_dir(path);
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let backup = dir.join(format!("{stem}-v{version}-{seconds}.db"));
    let target = backup
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", backup.display()))?;
    connection
        .execute("VACUUM INTO ?1", [target])
        .with_context(|| format!("cannot back up {} to {}", path.display(), backup.display()))?;
    Ok(())
}

fn backups_dir(path: &Path) -> PathBuf {
    path.with_file_name("backups")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;
    use rusqlite::types::FromSql;
    use rusqlite_migration::{M, Migrations};
    use tempfile::TempDir;

    use super::{migrations, open};

    const PROJECT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XA";
    const OTHER_PROJECT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XB";
    const ACCOUNT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XC";
    const OTHER_ACCOUNT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XD";

    fn open_latest(dir: &TempDir) -> Connection {
        open(&dir.path().join("cloneyard.db"), migrations()).unwrap()
    }

    fn v1() -> Vec<M<'static>> {
        migrations().into_iter().take(1).collect()
    }

    fn pragma<T: FromSql>(connection: &Connection, name: &str) -> T {
        connection
            .pragma_query_value(None, name, |row| row.get(0))
            .unwrap()
    }

    fn backups(dir: &TempDir) -> Vec<PathBuf> {
        match fs::read_dir(dir.path().join("backups")) {
            Ok(entries) => entries.map(|entry| entry.unwrap().path()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn unix_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn repos(connection: &Connection) -> Vec<(String, String)> {
        connection
            .prepare("SELECT project.name, repo.name FROM repo JOIN project ON project.id = repo.project_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn with_projects(dir: &TempDir) -> Connection {
        let connection = open_latest(dir);
        connection
            .execute(
                "INSERT INTO project (id, name) VALUES (?1, 'cloneyard'), (?2, 'website')",
                [PROJECT, OTHER_PROJECT],
            )
            .unwrap();
        connection
    }

    fn add_repo(
        connection: &Connection,
        id: &str,
        project_id: &str,
        name: &str,
        remote_url: Option<&str>,
    ) -> rusqlite::Result<usize> {
        connection.execute(
            "INSERT INTO repo (id, project_id, name, remote_url) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, project_id, name, remote_url],
        )
    }

    #[test]
    fn migrations_are_valid() {
        Migrations::new(migrations()).validate().unwrap();
    }

    #[test]
    fn creates_a_missing_file_at_the_latest_schema_with_wal_and_foreign_keys_on() {
        let dir = TempDir::new().unwrap();

        let connection = open_latest(&dir);

        assert_eq!(pragma::<i64>(&connection, "user_version"), 4);
        assert_eq!(pragma::<String>(&connection, "journal_mode"), "wal");
        assert_eq!(pragma::<i64>(&connection, "foreign_keys"), 1);
        assert!(!dir.path().join("backups").exists());
    }

    #[test]
    fn backs_up_a_v1_database_before_migrating_it_and_keeps_its_rows() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cloneyard.db");
        let connection = open(&path, v1()).unwrap();
        connection
            .execute(
                "INSERT INTO project (id, name) VALUES (?1, 'cloneyard')",
                [PROJECT],
            )
            .unwrap();
        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0",
            PROJECT,
            "api",
            None,
        )
        .unwrap();
        drop(connection);
        let before = unix_seconds();

        let connection = open(&path, migrations()).unwrap();

        let after = unix_seconds();
        let expected = vec![("cloneyard".to_owned(), "api".to_owned())];
        assert_eq!(pragma::<i64>(&connection, "user_version"), 4);
        let paths: i64 = connection
            .query_row("SELECT count(*) FROM repo_path", [], |row| row.get(0))
            .unwrap();
        assert_eq!(paths, 0);
        assert_eq!(repos(&connection), expected);
        let [backup] = backups(&dir).try_into().unwrap();
        let name = backup.file_name().unwrap().to_str().unwrap();
        let seconds: u64 = name
            .strip_prefix("cloneyard-v1-")
            .and_then(|rest| rest.strip_suffix(".db"))
            .and_then(|seconds| seconds.parse().ok())
            .unwrap_or_else(|| panic!("unexpected backup name {name}"));
        assert!((before..=after).contains(&seconds), "{name}");
        let backup = Connection::open(&backup).unwrap();
        assert_eq!(pragma::<i64>(&backup, "user_version"), 1);
        assert_eq!(repos(&backup), expected);
    }

    #[test]
    fn backs_up_a_v3_database_before_adding_the_account_tables() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cloneyard.db");
        let v3: Vec<M<'static>> = migrations().into_iter().take(3).collect();
        let connection = open(&path, v3).unwrap();
        connection
            .execute(
                "INSERT INTO project (id, name) VALUES (?1, 'cloneyard')",
                [PROJECT],
            )
            .unwrap();
        drop(connection);

        let connection = open(&path, migrations()).unwrap();

        assert_eq!(pragma::<i64>(&connection, "user_version"), 4);
        let counts: (i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT count(*) FROM project), (SELECT count(*) FROM account), \
                 (SELECT count(*) FROM account_dir)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (1, 0, 0));
        let [backup] = backups(&dir).try_into().unwrap();
        let name = backup.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("cloneyard-v3-"), "{name}");
    }

    #[test]
    fn an_account_dir_belongs_to_one_account_on_a_node() {
        let dir = TempDir::new().unwrap();
        let connection = open_latest(&dir);
        connection
            .execute(
                "INSERT INTO account (id, name, provider) \
                 VALUES (?1, 'personal', 'claude-code'), (?2, 'work', 'claude-code')",
                [ACCOUNT, OTHER_ACCOUNT],
            )
            .unwrap();
        let add_dir = |account_id: &str, node_id: &str| {
            connection.execute(
                "INSERT INTO account_dir (account_id, node_id, dir) VALUES (?1, ?2, '/data/claude')",
                [account_id, node_id],
            )
        };
        add_dir(ACCOUNT, "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0").unwrap();

        let error = add_dir(OTHER_ACCOUNT, "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0").unwrap_err();
        add_dir(OTHER_ACCOUNT, "01K9Z3V6Y8Q2N5M7P4R1T0W3Y1").unwrap();

        assert_eq!(
            error.to_string(),
            "UNIQUE constraint failed: account_dir.node_id, account_dir.dir"
        );
    }

    #[test]
    fn refuses_a_schema_newer_than_its_migrations_and_leaves_the_file_alone() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cloneyard.db");
        let mut newer = migrations();
        newer.push(M::up("ALTER TABLE repo ADD COLUMN default_branch TEXT;"));
        newer.push(M::up("ALTER TABLE repo ADD COLUMN archived INTEGER;"));
        drop(open(&path, newer).unwrap());

        let error = open(&path, migrations()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "database schema v6 is newer than this cloneyard (v4); \
                 upgrade cloneyard or restore a file from {}",
                dir.path().join("backups").display()
            )
        );
        let connection = Connection::open(&path).unwrap();
        assert_eq!(pragma::<i64>(&connection, "user_version"), 6);
    }

    #[test]
    fn reopening_an_up_to_date_database_makes_no_backup() {
        let dir = TempDir::new().unwrap();
        drop(open_latest(&dir));

        drop(open_latest(&dir));

        assert_eq!(backups(&dir), Vec::<PathBuf>::new());
    }

    #[test]
    fn names_the_file_when_it_is_not_a_database() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cloneyard.db");
        fs::write(
            &path,
            "not a database, just text long enough to fill a header\n",
        )
        .unwrap();

        let error = open(&path, migrations()).unwrap_err();

        assert_eq!(error.to_string(), format!("cannot open {}", path.display()));
        assert!(
            format!("{error:#}").contains("file is not a database"),
            "{error:#}"
        );
    }

    #[test]
    fn the_change_log_takes_only_upserts_and_deletes() {
        let dir = TempDir::new().unwrap();
        let connection = open_latest(&dir);
        let log = |id: &str, op: &str| {
            connection.execute(
                "INSERT INTO change_log (id, node_id, hlc, entity, entity_id, op, fields) \
                 VALUES (?1, ?2, 1, 'project', ?3, ?4, '{}')",
                [id, OTHER_PROJECT, PROJECT, op],
            )
        };

        log("01K9Z3V6Y8Q2N5M7P4R1T0W3Y0", "upsert").unwrap();
        log("01K9Z3V6Y8Q2N5M7P4R1T0W3Y1", "delete").unwrap();
        let error = log("01K9Z3V6Y8Q2N5M7P4R1T0W3Y2", "append").unwrap_err();

        assert_eq!(
            error.to_string(),
            "CHECK constraint failed: op IN ('upsert', 'delete')"
        );
    }

    #[test]
    fn a_repo_needs_an_existing_project() {
        let dir = TempDir::new().unwrap();
        let connection = with_projects(&dir);

        let error = add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0",
            "01K9Z3V6Y8Q2N5M7P4R1T0W3XZ",
            "cloneyard",
            None,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "FOREIGN KEY constraint failed");
    }

    #[test]
    fn a_remote_url_belongs_to_one_repo() {
        let dir = TempDir::new().unwrap();
        let connection = with_projects(&dir);
        let url = Some("git@github.com:Cloneyard/cloneyard.git");
        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0",
            PROJECT,
            "cloneyard",
            url,
        )
        .unwrap();

        let error = add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y1",
            OTHER_PROJECT,
            "copy",
            url,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "UNIQUE constraint failed: repo.remote_url"
        );
    }

    #[test]
    fn repos_without_a_remote_url_can_share_a_project() {
        let dir = TempDir::new().unwrap();
        let connection = with_projects(&dir);

        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0",
            PROJECT,
            "api",
            None,
        )
        .unwrap();
        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y1",
            PROJECT,
            "web",
            None,
        )
        .unwrap();
    }

    #[test]
    fn a_repo_name_is_unique_within_its_project() {
        let dir = TempDir::new().unwrap();
        let connection = with_projects(&dir);
        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y0",
            PROJECT,
            "api",
            None,
        )
        .unwrap();

        let error = add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y1",
            PROJECT,
            "api",
            None,
        )
        .unwrap_err();
        add_repo(
            &connection,
            "01K9Z3V6Y8Q2N5M7P4R1T0W3Y2",
            OTHER_PROJECT,
            "api",
            None,
        )
        .unwrap();

        assert_eq!(
            error.to_string(),
            "UNIQUE constraint failed: repo.project_id, repo.name"
        );
    }
}
