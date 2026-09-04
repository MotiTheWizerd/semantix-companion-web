mod repository;

use std::{path::Path, sync::Arc};

pub(crate) use repository::{PreferenceRepository, ResolvedVoice};
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
    /// A Claude Code model (SDK alias like "opus"/"sonnet") — not a row in
    /// configured_models. Selectable today; the chat path refuses it until the
    /// Claude Code integration lands.
    #[serde(rename = "claude_code")]
    ClaudeCode {
        #[serde(rename = "modelId")]
        model_id: String,
    },
}

impl ModelPreference {
    pub(crate) fn from_storage(mode: &str, model_id: Option<String>) -> Self {
        match (mode, model_id) {
            ("inherit", _) => Self::Inherit,
            ("configured", Some(model_id)) => Self::Configured { model_id },
            ("claude_code", Some(model_id)) => Self::ClaudeCode { model_id },
            _ => Self::Test,
        }
    }

    pub(crate) fn storage_parts(&self) -> (&'static str, Option<&str>) {
        match self {
            Self::Inherit => ("inherit", None),
            Self::Test => ("test", None),
            Self::Configured { model_id } => ("configured", Some(model_id.trim())),
            Self::ClaudeCode { model_id } => ("claude_code", Some(model_id.trim())),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserPreferences {
    default_model: ModelPreference,
    /// What to call the person using this install. None until they say — the
    /// interface falls back to "You" rather than guessing.
    display_name: Option<String>,
    updated_at: i64,
}

/// A PATCH, not a replacement: every field is optional and an absent one is
/// left exactly as it was. The alternative — one input carrying the whole row —
/// would make renaming yourself re-send the default model, so a stale copy in
/// one screen could silently revert a change made in another.
///
/// `display_name: Some("")` is meaningful: it CLEARS the name (stored NULL).
/// Absent leaves it alone.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUserPreferencesInput {
    #[serde(default)]
    default_model: Option<ModelPreference>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum UserPreferencesChangedEvent {
    Updated { preferences: UserPreferences },
}

impl UserPreferences {
    /// What to call the person, for anything that speaks about them. None when
    /// they have not said — and a caller that gets None must say something
    /// other than a guess.
    pub(crate) fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }
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
        repository.update_user_preferences(
            input.default_model.as_ref(),
            input.display_name.as_deref(),
            unix_timestamp_ms()?,
        )
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

        let claude = serde_json::to_value(ModelPreference::ClaudeCode {
            model_id: "opus".to_owned(),
        })
        .expect("claude code preference should serialize");
        assert_eq!(claude["mode"], "claude_code");
        assert_eq!(claude["modelId"], "opus");
    }

    #[test]
    fn claude_code_preferences_survive_the_storage_roundtrip() {
        let preference = ModelPreference::ClaudeCode {
            model_id: "sonnet".to_owned(),
        };
        let (mode, model_id) = preference.storage_parts();
        assert_eq!(mode, "claude_code");
        let restored = ModelPreference::from_storage(mode, model_id.map(str::to_owned));
        assert_eq!(restored, preference);
    }
}
