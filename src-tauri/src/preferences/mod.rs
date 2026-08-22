mod repository;

use std::{path::Path, sync::Arc};

pub(crate) use repository::PreferenceRepository;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::{app_error::AppError, credentials::unix_timestamp_ms};

const USER_PREFERENCES_CHANGED_EVENT: &str = "preferences://changed";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub(crate) enum ModelPreference {
    Inherit,
    Test,
    Configured {
        #[serde(rename = "modelId")]
        model_id: String,
    },
}

impl ModelPreference {
    pub(crate) fn from_storage(mode: &str, model_id: Option<String>) -> Self {
        match (mode, model_id) {
            ("inherit", _) => Self::Inherit,
            ("configured", Some(model_id)) => Self::Configured { model_id },
            _ => Self::Test,
        }
    }

    pub(crate) fn storage_parts(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Inherit => ("inherit", None),
            Self::Test => ("test", None),
            Self::Configured { model_id } => ("configured", Some(model_id.trim())),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserPreferences {
    default_model: ModelPreference,
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUserPreferencesInput {
    default_model: ModelPreference,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum UserPreferencesChangedEvent {
    Updated { preferences: UserPreferences },
}

pub(crate) struct PreferenceState {
    repository: Arc<PreferenceRepository>,
}

impl PreferenceState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            repository: Arc::new(PreferenceRepository::open(database_path)?),
        })
    }
}

#[tauri::command]
pub(crate) async fn get_user_preferences(
    state: State<'_, PreferenceState>,
) -> Result<UserPreferences, String> {
    let repository = Arc::clone(&state.repository);
    tauri::async_runtime::spawn_blocking(move || repository.get_user_preferences())
        .await
        .map_err(|error| format!("Preference task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn update_user_preferences(
    app: AppHandle,
    state: State<'_, PreferenceState>,
    input: UpdateUserPreferencesInput,
) -> Result<UserPreferences, String> {
    let repository = Arc::clone(&state.repository);
    let preferences = tauri::async_runtime::spawn_blocking(move || {
        repository.update_user_preferences(&input.default_model, unix_timestamp_ms()?)
    })
    .await
    .map_err(|error| format!("Preference task failed: {error}"))?
    .map_err(String::from)?;

    let _ = app.emit(
        USER_PREFERENCES_CHANGED_EVENT,
        UserPreferencesChangedEvent::Updated {
            preferences: preferences.clone(),
        },
    );
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::ModelPreference;

    #[test]
    fn model_preferences_use_an_explicit_tagged_contract() {
        let configured = serde_json::to_value(ModelPreference::Configured {
            model_id: "model-123".to_owned(),
        })
        .expect("configured preference should serialize");
        assert_eq!(configured["mode"], "configured");
        assert_eq!(configured["modelId"], "model-123");

        let inherit = serde_json::to_value(ModelPreference::Inherit)
            .expect("inherit preference should serialize");
        assert_eq!(inherit["mode"], "inherit");
    }
}
