use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{Style, StyleExemplar};
use crate::{app_error::AppError, database};

const STYLE_COLUMNS: &str = "s.id, s.name, s.description, s.style_card, s.created_at, s.updated_at,
     (SELECT COUNT(*) FROM style_exemplars e WHERE e.style_id = s.id)";

pub(crate) struct StyleRepository {
    connection: Mutex<Connection>,
}

impl StyleRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    pub(crate) fn list(&self) -> Result<Vec<Style>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {STYLE_COLUMNS} FROM styles s ORDER BY s.created_at ASC, s.id ASC"
            ))
            .map_err(AppError::database)?;
        let styles = statement
            .query_map([], map_style)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(styles)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Style>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                &format!("SELECT {STYLE_COLUMNS} FROM styles s WHERE s.id = ?1"),
                [id],
                map_style,
            )
            .optional()
            .map_err(AppError::database)
    }

    /// The exemplars the prompt builder actually sends, in curated order.
    /// `limit` is the prompt budget — the table may hold far more than any
    /// one request should carry.
    pub(crate) fn exemplars(
        &self,
        style_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<StyleExemplar>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, position, user_text, companion_text, era
                 FROM style_exemplars
                 WHERE style_id = ?1
                 ORDER BY position ASC, id ASC
                 LIMIT ?2",
            )
            .map_err(AppError::database)?;
        let limit = limit.map(|n| n as i64).unwrap_or(-1);
        let exemplars = statement
            .query_map(params![style_id, limit], |row| {
                Ok(StyleExemplar {
                    id: row.get(0)?,
                    position: row.get(1)?,
                    user_text: row.get(2)?,
                    companion_text: row.get(3)?,
                    era: row.get(4)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(exemplars)
    }

    pub(crate) fn insert(&self, style: &Style, exemplars: &[StyleExemplar]) -> Result<(), AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute(
                "INSERT INTO styles (id, name, description, style_card, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    style.id,
                    style.name,
                    style.description,
                    style.style_card,
                    style.created_at,
                    style.updated_at,
                ],
            )
            .map_err(AppError::database)?;
        insert_exemplars(&transaction, &style.id, exemplars)?;
        transaction.commit().map_err(AppError::database)?;
        Ok(())
    }

    /// A style update replaces the exemplar set wholesale when one is given.
    /// The set is the curated artifact the editor works on — patching single
    /// rows would make position bookkeeping the caller's problem.
    pub(crate) fn update(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        style_card: Option<&str>,
        exemplars: Option<&[StyleExemplar]>,
        updated_at: i64,
    ) -> Result<(), AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute(
                "UPDATE styles
                 SET name = ?2, description = ?3, style_card = ?4, updated_at = ?5
                 WHERE id = ?1",
                params![id, name, description, style_card, updated_at],
            )
            .map_err(AppError::database)?;
        if let Some(exemplars) = exemplars {
            transaction
                .execute("DELETE FROM style_exemplars WHERE style_id = ?1", [id])
                .map_err(AppError::database)?;
            insert_exemplars(&transaction, id, exemplars)?;
        }
        transaction.commit().map_err(AppError::database)?;
        Ok(())
    }

    pub(crate) fn delete(&self, id: &str) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute("DELETE FROM styles WHERE id = ?1", [id])
            .map_err(AppError::database)?;
        Ok(())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}

fn map_style(row: &rusqlite::Row<'_>) -> rusqlite::Result<Style> {
    Ok(Style {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        style_card: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        exemplar_count: row.get(6)?,
    })
}

fn insert_exemplars(
    connection: &Connection,
    style_id: &str,
    exemplars: &[StyleExemplar],
) -> Result<(), AppError> {
    for (position, exemplar) in exemplars.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO style_exemplars (
                     id, style_id, position, user_text, companion_text, era
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    exemplar.id,
                    style_id,
                    position as i64,
                    exemplar.user_text,
                    exemplar.companion_text,
                    exemplar.era,
                ],
            )
            .map_err(AppError::database)?;
    }
    Ok(())
}
