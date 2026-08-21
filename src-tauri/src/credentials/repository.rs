use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::CredentialMetadata;
use crate::{app_error::AppError, database};

#[derive(Clone, Debug)]
pub(crate) struct CredentialRecord {
    pub metadata: CredentialMetadata,
    pub secret_ref: String,
}

pub(crate) struct CredentialRepository {
    connection: Mutex<Connection>,
}

impl CredentialRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    pub(crate) fn list(&self) -> Result<Vec<CredentialMetadata>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, provider_id, label, key_hint, created_at, updated_at, last_used_at
                 FROM provider_credentials
                 ORDER BY updated_at DESC, label COLLATE NOCASE ASC",
            )
            .map_err(AppError::database)?;

        let credentials = statement
            .query_map([], |row| {
                Ok(CredentialMetadata {
                    id: row.get(0)?,
                    provider_id: row.get(1)?,
                    label: row.get(2)?,
                    key_hint: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    last_used_at: row.get(6)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(credentials)
    }

    pub(crate) fn insert(&self, record: &CredentialRecord) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO provider_credentials (
                    id, provider_id, label, secret_ref, key_hint,
                    created_at, updated_at, last_used_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.metadata.id,
                    record.metadata.provider_id,
                    record.metadata.label,
                    record.secret_ref,
                    record.metadata.key_hint,
                    record.metadata.created_at,
                    record.metadata.updated_at,
                    record.metadata.last_used_at,
                ],
            )
            .map_err(AppError::database)?;

        Ok(())
    }

    pub(crate) fn update(&self, record: &CredentialRecord) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE provider_credentials
                 SET provider_id = ?2,
                     label = ?3,
                     key_hint = ?4,
                     updated_at = ?5,
                     last_used_at = ?6
                 WHERE id = ?1",
                params![
                    record.metadata.id,
                    record.metadata.provider_id,
                    record.metadata.label,
                    record.metadata.key_hint,
                    record.metadata.updated_at,
                    record.metadata.last_used_at,
                ],
            )
            .map_err(AppError::database)?;

        Ok(())
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<CredentialRecord>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, provider_id, label, secret_ref, key_hint,
                        created_at, updated_at, last_used_at
                 FROM provider_credentials
                 WHERE id = ?1",
                [id],
                |row| {
                    Ok(CredentialRecord {
                        metadata: CredentialMetadata {
                            id: row.get(0)?,
                            provider_id: row.get(1)?,
                            label: row.get(2)?,
                            key_hint: row.get(4)?,
                            created_at: row.get(5)?,
                            updated_at: row.get(6)?,
                            last_used_at: row.get(7)?,
                        },
                        secret_ref: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(AppError::database)
    }

    pub(crate) fn is_used_by_model(&self, id: &str) -> Result<bool, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM configured_models WHERE credential_id = ?1
                 )",
                [id],
                |row| row.get(0),
            )
            .map_err(AppError::database)
    }

    pub(crate) fn delete(&self, id: &str) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute("DELETE FROM provider_credentials WHERE id = ?1", [id])
            .map_err(AppError::database)?;
        Ok(())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}
