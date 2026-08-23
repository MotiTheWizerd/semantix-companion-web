use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::Companion;
use crate::{app_error::AppError, database};

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
            .prepare(
                "SELECT id, name, memory_agent_name, is_built_in, created_at, updated_at
                 FROM companions
                 ORDER BY is_built_in DESC, created_at ASC, id ASC",
            )
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
                "SELECT id, name, memory_agent_name, is_built_in, created_at, updated_at
                 FROM companions
                 WHERE id = ?1",
                [id],
                map_companion,
            )
            .optional()
            .map_err(AppError::database)
    }

    pub(crate) fn insert(&self, companion: &Companion) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO companions (
                    id, name, memory_agent_name, is_built_in, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    companion.id,
                    companion.name,
                    companion.memory_agent_name,
                    i64::from(companion.is_built_in),
                    companion.created_at,
                    companion.updated_at,
                ],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    /// Only the name is writable. `memory_agent_name` is the companion's private
    /// memory and stays put for the life of the record.
    pub(crate) fn rename(
        &self,
        id: &str,
        name: Option<&str>,
        updated_at: i64,
    ) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE companions SET name = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, name, updated_at],
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
    Ok(Companion {
        id: row.get(0)?,
        name: row.get(1)?,
        memory_agent_name: row.get(2)?,
        is_built_in: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}
