use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;
use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

pub(crate) fn migrations() -> Vec<M<'static>> {
    vec![M::up(include_str!("../migrations/0001-initial.sql"))]
}

pub(crate) fn open(path: &Path, migrations: Vec<M<'static>>) -> anyhow::Result<Connection> {
    let mut connection =
        connect(path).with_context(|| format!("cannot open {}", path.display()))?;
    Migrations::new(migrations)
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

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;
    use rusqlite::types::FromSql;
    use rusqlite_migration::Migrations;
    use tempfile::TempDir;

    use super::{migrations, open};

    const PROJECT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XA";
    const OTHER_PROJECT: &str = "01K9Z3V6Y8Q2N5M7P4R1T0W3XB";

    fn open_latest(dir: &TempDir) -> Connection {
        open(&dir.path().join("cloneyard.db"), migrations()).unwrap()
    }

    fn pragma<T: FromSql>(connection: &Connection, name: &str) -> T {
        connection
            .pragma_query_value(None, name, |row| row.get(0))
            .unwrap()
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

        assert_eq!(pragma::<i64>(&connection, "user_version"), 1);
        assert_eq!(pragma::<String>(&connection, "journal_mode"), "wal");
        assert_eq!(pragma::<i64>(&connection, "foreign_keys"), 1);
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
