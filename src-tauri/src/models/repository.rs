use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{ConfiguredModel, ModelRecord};
use crate::app_error::AppError;

pub(crate) struct ModelRepository {
    connection: Mutex<Connection>,
}

impl ModelRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        let connection = Connection::open(path).map_err(AppError::database)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(AppError::database)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA foreign_keys = ON;

                 CREATE TABLE IF NOT EXISTS configured_models (
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
                     ON configured_models(credential_id);

                 PRAGMA user_version = 2;",
            )
            .map_err(AppError::database)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub(crate) fn list(&self) -> Result<Vec<ConfiguredModel>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT
                    models.id,
                    models.provider_id,
                    models.model_id,
                    models.display_name,
                    models.credential_id,
                    CASE WHEN models.credential_id IS NULL THEN 'manual' ELSE 'saved' END,
                    COALESCE(credentials.label, 'Manual key'),
                    COALESCE(credentials.key_hint, models.manual_key_hint),
                    models.created_at,
                    models.updated_at
                 FROM configured_models AS models
                 LEFT JOIN provider_credentials AS credentials
                    ON credentials.id = models.credential_id
                 ORDER BY models.updated_at DESC, models.display_name COLLATE NOCASE ASC",
            )
            .map_err(AppError::database)?;

        let models = statement
            .query_map([], |row| {
                Ok(ConfiguredModel {
                    id: row.get(0)?,
                    provider_id: row.get(1)?,
                    model_id: row.get(2)?,
                    display_name: row.get(3)?,
                    credential_id: row.get(4)?,
                    credential_kind: row.get(5)?,
                    credential_label: row.get(6)?,
                    key_hint: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(models)
    }

    pub(crate) fn insert(&self, record: &ModelRecord) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO configured_models (
                    id, provider_id, model_id, display_name, credential_id,
                    secret_ref, manual_key_hint, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.model.id,
                    record.model.provider_id,
                    record.model.model_id,
                    record.model.display_name,
                    record.model.credential_id,
                    record.secret_ref,
                    record.manual_key_hint,
                    record.model.created_at,
                    record.model.updated_at,
                ],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<ModelRecord>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT
                    models.id,
                    models.provider_id,
                    models.model_id,
                    models.display_name,
                    models.credential_id,
                    CASE WHEN models.credential_id IS NULL THEN 'manual' ELSE 'saved' END,
                    COALESCE(credentials.label, 'Manual key'),
                    COALESCE(credentials.key_hint, models.manual_key_hint),
                    models.created_at,
                    models.updated_at,
                    models.secret_ref,
                    models.manual_key_hint
                 FROM configured_models AS models
                 LEFT JOIN provider_credentials AS credentials
                    ON credentials.id = models.credential_id
                 WHERE models.id = ?1",
                [id],
                |row| {
                    Ok(ModelRecord {
                        model: ConfiguredModel {
                            id: row.get(0)?,
                            provider_id: row.get(1)?,
                            model_id: row.get(2)?,
                            display_name: row.get(3)?,
                            credential_id: row.get(4)?,
                            credential_kind: row.get(5)?,
                            credential_label: row.get(6)?,
                            key_hint: row.get(7)?,
                            created_at: row.get(8)?,
                            updated_at: row.get(9)?,
                        },
                        secret_ref: row.get(10)?,
                        manual_key_hint: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(AppError::database)
    }

    pub(crate) fn delete(&self, id: &str) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute("DELETE FROM configured_models WHERE id = ?1", [id])
            .map_err(AppError::database)?;
        Ok(())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}
