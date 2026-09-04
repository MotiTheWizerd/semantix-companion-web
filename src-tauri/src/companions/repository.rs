use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{avatars, Companion, CompanionWorkspace, OriginIdentity};
use crate::{app_error::AppError, database, preferences::ModelPreference};

const COMPANION_COLUMNS: &str = "id, name, memory_agent_name, model_preference_mode, model_id,
     is_built_in, created_at, updated_at, is_origin, origin_agent_id, style_id, avatar_file";

pub(crate) struct CompanionRepository {
    connection: Mutex<Connection>,
    /// Where the faces are. Derived from the database path at open time rather
    /// than resolved independently, so the pictures cannot end up beside a
    /// different database than the rows that name them.
    avatars_dir: PathBuf,
}

impl CompanionRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
            avatars_dir: avatars::directory(path),
        })
    }

    pub(crate) fn avatars_dir(&self) -> &Path {
        &self.avatars_dir
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

        let avatars_dir = self.avatars_dir.clone();
        let mut companions = statement
            .query_map([], move |row| map_companion(row, &avatars_dir))
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        drop(statement);

        let mut workspaces = load_all_workspaces(&connection)?;
        for companion in &mut companions {
            companion.workspaces = workspaces.remove(&companion.id).unwrap_or_default();
        }

        Ok(companions)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Companion>, AppError> {
        let connection = self.connection()?;
        let mut companion = connection
            .query_row(
                &format!("SELECT {COMPANION_COLUMNS} FROM companions WHERE id = ?1"),
                [id],
                |row| map_companion(row, &self.avatars_dir),
            )
            .optional()
            .map_err(AppError::database)?;
        if let Some(companion) = &mut companion {
            companion.workspaces = load_workspaces(&connection, &companion.id)?;
        }
        Ok(companion)
    }

    /// The seeded companion — the fallback whenever a thread has no companion
    /// of its own. Guaranteed by the schema to exist and be unique.
    pub(crate) fn built_in(&self) -> Result<Option<Companion>, AppError> {
        let connection = self.connection()?;
        let mut companion = connection
            .query_row(
                &format!("SELECT {COMPANION_COLUMNS} FROM companions WHERE is_built_in = 1"),
                [],
                |row| map_companion(row, &self.avatars_dir),
            )
            .optional()
            .map_err(AppError::database)?;
        if let Some(companion) = &mut companion {
            companion.workspaces = load_workspaces(&connection, &companion.id)?;
        }
        Ok(companion)
    }

    /// Which brain the companion owning this agent name reads from.
    ///
    /// Keyed on `memory_agent_name` rather than the companion id because the
    /// memory calls only ever carry the agent, never the companion. A miss is
    /// `false`: an agent this roster has never heard of is not one of ours to
    /// point at the local Muninn, so an unknown name fails safely toward the
    /// organ every install can reach.
    pub(crate) fn origin_identity(
        &self,
        memory_agent_name: &str,
    ) -> Result<Option<OriginIdentity>, AppError> {
        let connection = self.connection()?;
        let row: Option<(i64, Option<String>)> = connection
            .query_row(
                "SELECT is_origin, origin_agent_id FROM companions WHERE memory_agent_name = ?1",
                [memory_agent_name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(AppError::database)?;
        Ok(match row {
            Some((origin, agent_id)) if origin != 0 => Some(OriginIdentity {
                agent_id: agent_id
                    .map(|id| id.trim().to_owned())
                    .filter(|id| !id.is_empty()),
            }),
            _ => None,
        })
    }

    pub(crate) fn insert(&self, companion: &Companion) -> Result<(), AppError> {
        let (mode, model_id) = companion.model_preference.storage_parts();
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute(
                "INSERT INTO companions (
                    id, name, memory_agent_name, model_preference_mode, model_id,
                    is_built_in, created_at, updated_at, is_origin, origin_agent_id, style_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    companion.id,
                    companion.name,
                    companion.memory_agent_name,
                    mode,
                    model_id,
                    i64::from(companion.is_built_in),
                    companion.created_at,
                    companion.updated_at,
                    i64::from(companion.is_origin),
                    companion.origin_agent_id,
                    companion.style_id,
                ],
            )
            .map_err(AppError::database)?;
        insert_workspaces(&transaction, &companion.id, &companion.workspaces)?;
        transaction.commit().map_err(AppError::database)?;
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
        workspaces: &[CompanionWorkspace],
        style_id: Option<&str>,
        updated_at: i64,
    ) -> Result<(), AppError> {
        let (mode, model_id) = model_preference.storage_parts();
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute(
                "UPDATE companions
                 SET name = ?2,
                     model_preference_mode = ?3,
                     model_id = ?4,
                     style_id = ?5,
                     updated_at = ?6
                 WHERE id = ?1",
                params![id, name, mode, model_id, style_id, updated_at],
            )
            .map_err(AppError::database)?;
        transaction
            .execute("DELETE FROM companion_workspaces WHERE companion_id = ?1", [id])
            .map_err(AppError::database)?;
        insert_workspaces(&transaction, id, workspaces)?;
        transaction.commit().map_err(AppError::database)?;
        Ok(())
    }

    /// Record (or clear) which file holds this companion's face.
    ///
    /// Separate from `update_details` because setting a picture is not an edit
    /// of the form: it comes from a file picker, it must not require the name,
    /// model and workspace fields to be round-tripped, and it must not be
    /// undone by a later Save of a form that was opened before the picture was
    /// chosen.
    pub(crate) fn set_avatar_file(
        &self,
        id: &str,
        avatar_file: Option<&str>,
        updated_at: i64,
    ) -> Result<(), AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE companions SET avatar_file = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, avatar_file, updated_at],
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

/// The row plus, when one is recorded and still on disk, the picture itself.
///
/// The stored column is a file name; what the UI receives is a renderable
/// `data:` URL, so no caller outside this module ever handles an avatar path.
fn map_companion(row: &rusqlite::Row<'_>, avatars_dir: &Path) -> rusqlite::Result<Companion> {
    let mode: String = row.get(3)?;
    let model_id: Option<String> = row.get(4)?;
    let avatar_file: Option<String> = row.get(11)?;
    Ok(Companion {
        id: row.get(0)?,
        name: row.get(1)?,
        memory_agent_name: row.get(2)?,
        model_preference: ModelPreference::from_storage(&mode, model_id),
        is_built_in: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        workspaces: Vec::new(),
        is_origin: row.get::<_, i64>(8)? != 0,
        origin_agent_id: row.get(9)?,
        style_id: row.get(10)?,
        avatar_url: avatar_file
            .as_deref()
            .and_then(|name| avatars::read_data_url(avatars_dir, name)),
    })
}

fn load_workspaces(
    connection: &Connection,
    companion_id: &str,
) -> Result<Vec<CompanionWorkspace>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT id, label, directory
             FROM companion_workspaces
             WHERE companion_id = ?1
             ORDER BY position ASC, id ASC",
        )
        .map_err(AppError::database)?;
    let workspaces = statement
        .query_map([companion_id], |row| {
            Ok(CompanionWorkspace {
                id: row.get(0)?,
                label: row.get(1)?,
                directory: row.get(2)?,
            })
        })
        .map_err(AppError::database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::database)?;
    Ok(workspaces)
}

fn load_all_workspaces(
    connection: &Connection,
) -> Result<HashMap<String, Vec<CompanionWorkspace>>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT companion_id, id, label, directory
             FROM companion_workspaces
             ORDER BY companion_id ASC, position ASC, id ASC",
        )
        .map_err(AppError::database)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CompanionWorkspace {
                    id: row.get(1)?,
                    label: row.get(2)?,
                    directory: row.get(3)?,
                },
            ))
        })
        .map_err(AppError::database)?;
    let mut grouped: HashMap<String, Vec<CompanionWorkspace>> = HashMap::new();
    for row in rows {
        let (companion_id, workspace) = row.map_err(AppError::database)?;
        grouped.entry(companion_id).or_default().push(workspace);
    }
    Ok(grouped)
}

fn insert_workspaces(
    connection: &Connection,
    companion_id: &str,
    workspaces: &[CompanionWorkspace],
) -> Result<(), AppError> {
    for (position, workspace) in workspaces.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO companion_workspaces (
                     id, companion_id, label, directory, position
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    workspace.id,
                    companion_id,
                    workspace.label,
                    workspace.directory,
                    position as i64,
                ],
            )
            .map_err(AppError::database)?;
    }
    Ok(())
}
