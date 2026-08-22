use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{ModelPreference, UserPreferences};
use crate::{app_error::AppError, database};

pub(crate) struct PreferenceRepository {
    connection: Mutex<Connection>,
}

impl PreferenceRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    pub(crate) fn get_user_preferences(&self) -> Result<UserPreferences, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT default_model_mode, default_model_id, updated_at
                 FROM user_preferences
                 WHERE id = 1",
                [],
                |row| {
                    let mode: String = row.get(0)?;
                    let model_id: Option<String> = row.get(1)?;
                    Ok(UserPreferences {
                        default_model: ModelPreference::from_storage(&mode, model_id),
                        updated_at: row.get(2)?,
                    })
                },
            )
            .map_err(AppError::database)
    }

    pub(crate) fn update_user_preferences(
        &self,
        default_model: &ModelPreference,
        updated_at: i64,
    ) -> Result<UserPreferences, AppError> {
        if matches!(default_model, ModelPreference::Inherit) {
            return Err(AppError::validation(
                "The user default model cannot inherit from another preference.",
            ));
        }
        self.validate_model_preference(default_model)?;
        let (mode, model_id) = default_model.storage_parts();
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE user_preferences
                 SET default_model_mode = ?1,
                     default_model_id = ?2,
                     updated_at = ?3
                 WHERE id = 1",
                params![mode, model_id, updated_at],
            )
            .map_err(AppError::database)?;

        Ok(UserPreferences {
            default_model: default_model.clone(),
            updated_at,
        })
    }

    pub(crate) fn validate_model_preference(
        &self,
        preference: &ModelPreference,
    ) -> Result<(), AppError> {
        let ModelPreference::Configured { model_id } = preference else {
            return Ok(());
        };
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(AppError::validation("Choose a configured model."));
        }
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT id FROM configured_models WHERE id = ?1",
                [model_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(AppError::database)?
            .is_some();
        if !exists {
            return Err(AppError::validation(
                "That configured model no longer exists.",
            ));
        }
        Ok(())
    }

    pub(crate) fn resolve_model_id(
        &self,
        preference: &ModelPreference,
    ) -> Result<Option<String>, AppError> {
        let effective = if matches!(preference, ModelPreference::Inherit) {
            self.get_user_preferences()?.default_model
        } else {
            preference.clone()
        };
        self.validate_model_preference(&effective)?;
        Ok(match effective {
            ModelPreference::Configured { model_id } => Some(model_id),
            ModelPreference::Inherit | ModelPreference::Test => None,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}
