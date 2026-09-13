//! Embedded, forward-only database migrations.

use rusqlite::{Connection, OptionalExtension};

use super::sqlite::SqliteError;

/// Latest schema version understood by this binary.
pub const LATEST_VERSION: i64 = 3;

#[derive(Clone, Copy)]
struct Migration {
    version: i64,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../../migrations/0002_catalog_provenance.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../../migrations/0003_catalog_dismissals.sql"),
    },
];

pub(super) fn migrate(connection: &mut Connection) -> Result<(), SqliteError> {
    migrate_with(connection, MIGRATIONS)
}

fn migrate_with(connection: &mut Connection, migrations: &[Migration]) -> Result<(), SqliteError> {
    let current = schema_version(connection)?;
    if current > LATEST_VERSION {
        return Err(SqliteError::NewerSchema {
            found: current,
            supported: LATEST_VERSION,
        });
    }

    for migration in migrations.iter().filter(|item| item.version > current) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations(version, applied_at) \
             VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
            [migration.version],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

pub(super) fn schema_version(connection: &Connection) -> Result<i64, SqliteError> {
    let migrations_exist = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master \
         WHERE type = 'table' AND name = 'schema_migrations')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !migrations_exist {
        return Ok(0);
    }

    connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .optional()
        .map(|value| value.flatten().unwrap_or(0))
        .map_err(SqliteError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_migration_rolls_back_schema_and_version() {
        let mut connection = Connection::open_in_memory().expect("database should open");
        let migrations = [
            Migration {
                version: 1,
                sql: include_str!("../../migrations/0001_initial.sql"),
            },
            Migration {
                version: 2,
                sql: "CREATE TABLE must_rollback(id INTEGER); INVALID SQL;",
            },
        ];

        assert!(migrate_with(&mut connection, &migrations).is_err());
        assert_eq!(schema_version(&connection).expect("version should load"), 1);
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master \
                 WHERE type = 'table' AND name = 'must_rollback')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .expect("schema should be queryable");
        assert!(!exists);
    }
}
