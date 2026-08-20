use std::{path::Path, time::Duration};

use rusqlite::Connection;

use crate::app_error::AppError;

const LATEST_SCHEMA_VERSION: i64 = 3;

pub(crate) fn initialise(path: &Path) -> Result<(), AppError> {
    let mut connection = open_connection(path)?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL;")
        .map_err(AppError::database)?;
    migrate(&mut connection)
}

pub(crate) fn open_connection(path: &Path) -> Result<Connection, AppError> {
    let connection = Connection::open(path).map_err(AppError::database)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(AppError::database)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(AppError::database)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection) -> Result<(), AppError> {
    let current_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(AppError::database)?;

    if current_version > LATEST_SCHEMA_VERSION {
        return Err(AppError::internal(format!(
            "the local database uses schema version {current_version}, but this build supports up to {LATEST_SCHEMA_VERSION}"
        )));
    }

    for version in (current_version + 1)..=LATEST_SCHEMA_VERSION {
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute_batch(migration_sql(version))
            .map_err(AppError::database)?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(AppError::database)?;
        transaction.commit().map_err(AppError::database)?;
    }

    Ok(())
}

fn migration_sql(version: i64) -> &'static str {
    match version {
        1 => {
            "CREATE TABLE IF NOT EXISTS provider_credentials (
                 id TEXT PRIMARY KEY,
                 provider_id TEXT NOT NULL,
                 label TEXT NOT NULL,
                 secret_ref TEXT NOT NULL UNIQUE,
                 key_hint TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 last_used_at INTEGER
             );

             CREATE INDEX IF NOT EXISTS idx_provider_credentials_provider
                 ON provider_credentials(provider_id);"
        }
        2 => {
            "CREATE TABLE IF NOT EXISTS configured_models (
                 id TEXT PRIMARY KEY,
                 provider_id TEXT NOT NULL,
                 model_id TEXT NOT NULL,
                 display_name TEXT NOT NULL,
                 credential_id TEXT,
                 secret_ref TEXT UNIQUE,
                 manual_key_hint TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 FOREIGN KEY (credential_id)
                     REFERENCES provider_credentials(id)
                     ON DELETE RESTRICT,
                 CHECK (
                     (credential_id IS NOT NULL AND secret_ref IS NULL AND manual_key_hint IS NULL)
                     OR
                     (credential_id IS NULL AND secret_ref IS NOT NULL AND manual_key_hint IS NOT NULL)
                 )
             );

             CREATE INDEX IF NOT EXISTS idx_configured_models_provider
                 ON configured_models(provider_id);

             CREATE INDEX IF NOT EXISTS idx_configured_models_credential
                 ON configured_models(credential_id);"
        }
        3 => {
            "CREATE TABLE IF NOT EXISTS conversations (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 selected_model_id TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 archived_at INTEGER,
                 FOREIGN KEY (selected_model_id)
                     REFERENCES configured_models(id)
                     ON DELETE SET NULL
             );

             CREATE INDEX IF NOT EXISTS idx_conversations_updated
                 ON conversations(updated_at DESC);

             CREATE TABLE IF NOT EXISTS messages (
                 id TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL,
                 sequence INTEGER NOT NULL CHECK (sequence >= 0),
                 role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
                 status TEXT NOT NULL CHECK (status IN ('pending', 'streaming', 'completed', 'failed', 'cancelled')),
                 content TEXT NOT NULL DEFAULT '',
                 provider_id TEXT,
                 model_id TEXT,
                 error_message TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 completed_at INTEGER,
                 FOREIGN KEY (conversation_id)
                     REFERENCES conversations(id)
                     ON DELETE CASCADE,
                 UNIQUE (conversation_id, sequence)
             );

             CREATE INDEX IF NOT EXISTS idx_messages_conversation_sequence
                 ON messages(conversation_id, sequence);"
        }
        _ => unreachable!("all schema versions must have migration SQL"),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{migrate, LATEST_SCHEMA_VERSION};

    #[test]
    fn migrations_create_the_current_schema() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");

        migrate(&mut connection).expect("migrations should succeed");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version should be readable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        for table in [
            "provider_credentials",
            "configured_models",
            "conversations",
            "messages",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("schema lookup should succeed");
            assert!(exists, "{table} should exist");
        }
    }

    #[test]
    fn migration_three_upgrades_an_existing_version_two_database() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE provider_credentials (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     label TEXT NOT NULL,
                     secret_ref TEXT NOT NULL UNIQUE,
                     key_hint TEXT NOT NULL,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     last_used_at INTEGER
                 );
                 CREATE TABLE configured_models (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     model_id TEXT NOT NULL,
                     display_name TEXT NOT NULL,
                     credential_id TEXT,
                     secret_ref TEXT UNIQUE,
                     manual_key_hint TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 INSERT INTO provider_credentials VALUES (
                     'credential-1', 'openai', 'Primary', 'secret-1', '•••• 1234', 1, 1, NULL
                 );
                 PRAGMA user_version = 2;",
            )
            .expect("legacy schema should be created");

        migrate(&mut connection).expect("version two should upgrade");

        let credential_label: String = connection
            .query_row(
                "SELECT label FROM provider_credentials WHERE id = 'credential-1'",
                [],
                |row| row.get(0),
            )
            .expect("existing data should survive the migration");
        assert_eq!(credential_label, "Primary");

        let chat_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN ('conversations', 'messages')",
                [],
                |row| row.get(0),
            )
            .expect("chat tables should be queryable");
        assert_eq!(chat_tables, 2);
    }
}
