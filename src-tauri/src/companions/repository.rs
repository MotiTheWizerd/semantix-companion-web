use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::Companion;
use crate::{app_error::AppError, database, preferences::ModelPreference};

const COMPANION_COLUMNS: &str = "id, name, memory_agent_name, model_preference_mode, model_id,
     is_built_in, created_at, updated_at";

pub(crate) struct CompanionRepository {
    connection: Mutex<Connection>,
}

impl CompanionRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    /// The built-in companion leads; the rest follow in the order they were made,
    /// so the roster never reshuffles under an edit.
    pub(crate) fn list(&self) -> Result<Vec<Companion>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COMPANION_COLUMNS}
                 FROM companions
                 ORDER BY is_built_in DESC, created_at ASC, id ASC"
            ))
            .map_err(AppError::database)?;

        let companions = statement
            .query_map([], map_companion)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(companions)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Companion>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                &format!("SELECT {COMPANION_COLUMNS} FROM companions WHERE id = ?1"),
                [id],
                map_companion,
            )
            .optional()
            .map_err(AppError::database)
    }

    /// The seeded companion — the fallback whenever a thread has no companion
    /// of its own. Guaranteed by the schema to exist and be unique.
    pub(crate) fn built_in(&self) -> Result<Option<Companion>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                &format!("SELECT {COMPANION_COLUMNS} FROM companions WHERE is_built_in = 1"),
                [],
                map_companion,
            )
            .optional()
            .map_err(AppError::database)
    }

    pub(crate) fn insert(&self, companion: &Companion) -> Result<(), AppError> {
        let (mode, model_id) = companion.model_preference.storage_parts();
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO companions (
                    id, name, memory_agent_name, model_preference_mode, model_id,
                    is_built_in, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    companion.id,
                    companion.name,
                    companion.memory_agent_name,
                    mode,
                    model_id,
                    i64::from(companion.is_built_in),
                    companion.created_at,
                    companion.updated_at,
                ],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    /// Name and model are writable; `memory_agent_name` is NOT. That column is
    /// the companion's private memory and stays put for the life of the record,
    /// which is why there is no general-purpose update on this repository.
    pub(crate) fn update_details(
        &self,
        id: &str,
        name: Option<&str>,
        model_preference: &ModelPreference,
        updated_at: i64,
    ) -> Result<(), AppError> {
        let (mode, model_id) = model_preference.storage_parts();
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE companions
                 SET name = ?2,
                     model_preference_mode = ?3,
                     model_id = ?4,
                     updated_at = ?5
                 WHERE id = ?1",
                params![id, name, mode, model_id, updated_at],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    pub(crate) fn delete(&self, id: &str) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute("DELETE FROM companions WHERE id = ?1", [id])
            .map_err(AppError::database)?;
        Ok(())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}

fn map_companion(row: &rusqlite::Row<'_>) -> rusqlite::Result<Companion> {
    let mode: String = row.get(3)?;
    let model_id: Option<String> = row.get(4)?;
    Ok(Companion {
        id: row.get(0)?,
        name: row.get(1)?,
        memory_agent_name: row.get(2)?,
        model_preference: ModelPreference::from_storage(&mode, model_id),
        is_built_in: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
