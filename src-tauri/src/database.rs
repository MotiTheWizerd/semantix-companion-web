use std::{path::Path, time::Duration};

use rusqlite::Connection;

use crate::app_error::AppError;

const LATEST_SCHEMA_VERSION: i64 = 5;

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
        4 => {
            "ALTER TABLE conversations
                 ADD COLUMN model_preference_mode TEXT NOT NULL DEFAULT 'test'
                 CHECK (model_preference_mode IN ('inherit', 'test', 'configured'));

             UPDATE conversations
                 SET model_preference_mode = CASE
                     WHEN selected_model_id IS NULL THEN 'test'
                     ELSE 'configured'
                 END;

             CREATE TABLE user_preferences (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 default_model_mode TEXT NOT NULL
                     CHECK (default_model_mode IN ('test', 'configured')),
                 default_model_id TEXT,
                 updated_at INTEGER NOT NULL,
                 FOREIGN KEY (default_model_id)
                     REFERENCES configured_models(id)
                     ON DELETE SET NULL
             );

             INSERT INTO user_preferences (
                 id, default_model_mode, default_model_id, updated_at
             ) VALUES (1, 'test', NULL, 0);"
        }
        5 => {
            // Full-text index over message content — the ground under the
            // model's search_conversations tool (its "raw memory" drill).
            // External-content FTS5: the index stores no second copy of the
            // text, and the triggers keep it in lockstep with messages.
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                 content,
                 content='messages',
                 content_rowid='rowid'
             );

             INSERT INTO messages_fts(rowid, content)
                 SELECT rowid, content FROM messages;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_insert
                 AFTER INSERT ON messages BEGIN
                     INSERT INTO messages_fts(rowid, content)
                         VALUES (new.rowid, new.content);
                 END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_delete
                 AFTER DELETE ON messages BEGIN
                     INSERT INTO messages_fts(messages_fts, rowid, content)
                         VALUES ('delete', old.rowid, old.content);
                 END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_update
                 AFTER UPDATE OF content ON messages BEGIN
                     INSERT INTO messages_fts(messages_fts, rowid, content)
                         VALUES ('delete', old.rowid, old.content);
                     INSERT INTO messages_fts(rowid, content)
                         VALUES (new.rowid, new.content);
                 END;"
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
            "user_preferences",
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

    #[test]
    fn migration_four_preserves_existing_model_selection_semantics() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE configured_models (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     model_id TEXT NOT NULL,
                     display_name TEXT NOT NULL,
                     credential_id TEXT,
                     secret_ref TEXT,
                     manual_key_hint TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE conversations (
                     id TEXT PRIMARY KEY,
                     title TEXT NOT NULL,
                     selected_model_id TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     archived_at INTEGER
                 );
                 CREATE TABLE messages (
                     id TEXT PRIMARY KEY,
                     conversation_id TEXT NOT NULL,
                     sequence INTEGER NOT NULL,
                     role TEXT NOT NULL,
                     status TEXT NOT NULL,
                     content TEXT NOT NULL DEFAULT '',
                     provider_id TEXT,
                     model_id TEXT,
                     error_message TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     completed_at INTEGER
                 );
                 INSERT INTO configured_models VALUES (
                     'model-1', 'test', 'test-model', 'Test', NULL,
                     'secret-1', '•••• 1234', 1, 1
                 );
                 INSERT INTO conversations VALUES (
                     'conversation-configured', 'Configured', 'model-1', 1, 1, NULL
                 );
                 INSERT INTO conversations VALUES (
                     'conversation-test', 'Test', NULL, 2, 2, NULL
                 );
                 PRAGMA user_version = 3;",
            )
            .expect("version three schema should be created");

        migrate(&mut connection).expect("version three should upgrade");

        let configured_mode: String = connection
            .query_row(
                "SELECT model_preference_mode FROM conversations
                 WHERE id = 'conversation-configured'",
                [],
                |row| row.get(0),
            )
            .expect("configured conversation should survive");
        let test_mode: String = connection
            .query_row(
                "SELECT model_preference_mode FROM conversations
                 WHERE id = 'conversation-test'",
                [],
                |row| row.get(0),
            )
            .expect("test conversation should survive");
        assert_eq!(configured_mode, "configured");
        assert_eq!(test_mode, "test");

        let default_mode: String = connection
            .query_row(
                "SELECT default_model_mode FROM user_preferences WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("default preference should be created");
        assert_eq!(default_mode, "test");
    }
}
