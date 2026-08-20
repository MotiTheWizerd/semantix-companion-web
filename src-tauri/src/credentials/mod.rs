pub(crate) mod repository;

use std::{
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use repository::{CredentialRecord, CredentialRepository};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{app_error::AppError, secret_vault::SecretVault};

pub(crate) const CREDENTIALS_CHANGED_EVENT: &str = "credentials://changed";

const PROVIDERS: &[(&str, &str, &str)] = &[
    ("openai", "OpenAI", "sk-proj-…"),
    ("anthropic", "Anthropic", "sk-ant-…"),
    ("google", "Google Gemini", "AIza…"),
    ("openrouter", "OpenRouter", "sk-or-v1-…"),
    ("groq", "Groq", "gsk_…"),
    ("mistral", "Mistral AI", "Enter API key"),
    ("together", "Together AI", "Enter API key"),
    ("fireworks", "Fireworks AI", "Enter API key"),
    ("deepseek", "DeepSeek", "sk-…"),
    ("xai", "xAI", "xai-…"),
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDefinition {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) key_placeholder: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialMetadata {
    pub(crate) id: String,
    pub(crate) provider_id: String,
    pub(crate) label: String,
    pub(crate) key_hint: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) last_used_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateProviderCredentialInput {
    provider_id: String,
    label: String,
    api_key: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CredentialChangedEvent {
    Created {
        credential: CredentialMetadata,
    },
    Deleted {
        #[serde(rename = "credentialId")]
        credential_id: String,
    },
}

pub(crate) struct CredentialState {
    service: Arc<CredentialService>,
}

impl CredentialState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(CredentialService {
                repository: CredentialRepository::open(database_path)?,
            }),
        })
    }
}

struct CredentialService {
    repository: CredentialRepository,
}

impl CredentialService {
    fn list(&self) -> Result<Vec<CredentialMetadata>, AppError> {
        self.repository.list()
    }

    fn create(&self, input: CreateProviderCredentialInput) -> Result<CredentialMetadata, AppError> {
        let CreateProviderCredentialInput {
            provider_id,
            label,
            api_key,
        } = input;
        let api_key = Zeroizing::new(api_key);
        let provider = provider_by_id(provider_id.trim())
            .ok_or_else(|| AppError::validation("Choose a supported model provider."))?;
        let label = if label.trim().is_empty() {
            provider.name.clone()
        } else {
            label.trim().to_owned()
        };

        if label.chars().count() > 80 {
            return Err(AppError::validation(
                "The key label must be 80 characters or fewer.",
            ));
        }

        let api_key = api_key.trim();
        if api_key.chars().count() < 8 {
            return Err(AppError::validation("Enter a complete provider API key."));
        }

        let id = Uuid::new_v4().to_string();
        let secret_ref = format!("provider-api-key:{id}");
        let key_hint = key_hint(api_key);
        let timestamp = unix_timestamp_ms()?;

        SecretVault::store(&secret_ref, api_key)?;

        let metadata = CredentialMetadata {
            id,
            provider_id: provider.id,
            label,
            key_hint,
            created_at: timestamp,
            updated_at: timestamp,
            last_used_at: None,
        };
        let record = CredentialRecord {
            metadata: metadata.clone(),
            secret_ref: secret_ref.clone(),
        };

        if let Err(error) = self.repository.insert(&record) {
            let _ = SecretVault::delete(&secret_ref);
            return Err(error);
        }

        Ok(metadata)
    }

    fn delete(&self, id: &str) -> Result<(), AppError> {
        let record = self
            .repository
            .get(id)?
            .ok_or_else(|| AppError::validation("That saved API key no longer exists."))?;
        if self.repository.is_used_by_model(id)? {
            return Err(AppError::validation(
                "This API key is used by a configured model. Remove that model first.",
            ));
        }

        self.repository.delete(id)?;
        if let Err(error) = SecretVault::delete(&record.secret_ref) {
            let _ = self.repository.insert(&record);
            return Err(error);
        }

        Ok(())
    }
}

#[tauri::command]
pub(crate) fn list_known_model_providers() -> Vec<ProviderDefinition> {
    PROVIDERS
        .iter()
        .map(|(id, name, key_placeholder)| ProviderDefinition {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            key_placeholder: (*key_placeholder).to_owned(),
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn list_provider_credentials(
    state: State<'_, CredentialState>,
) -> Result<Vec<CredentialMetadata>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.list())
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn create_provider_credential(
    app: AppHandle,
    state: State<'_, CredentialState>,
    input: CreateProviderCredentialInput,
) -> Result<CredentialMetadata, String> {
    let service = Arc::clone(&state.service);
    let credential = tauri::async_runtime::spawn_blocking(move || service.create(input))
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        CREDENTIALS_CHANGED_EVENT,
        CredentialChangedEvent::Created {
            credential: credential.clone(),
        },
    );

    Ok(credential)
}

#[tauri::command]
pub(crate) async fn delete_provider_credential(
    app: AppHandle,
    state: State<'_, CredentialState>,
    credential_id: String,
) -> Result<(), String> {
    let service = Arc::clone(&state.service);
    let id = credential_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.delete(&id))
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        CREDENTIALS_CHANGED_EVENT,
        CredentialChangedEvent::Deleted { credential_id },
    );

    Ok(())
}

pub(crate) fn provider_by_id(id: &str) -> Option<ProviderDefinition> {
    PROVIDERS
        .iter()
        .find(|(provider_id, _, _)| *provider_id == id)
        .map(|(provider_id, name, key_placeholder)| ProviderDefinition {
            id: (*provider_id).to_owned(),
            name: (*name).to_owned(),
            key_placeholder: (*key_placeholder).to_owned(),
        })
}

pub(crate) fn key_hint(api_key: &str) -> String {
    let suffix = api_key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("•••• {suffix}")
}

pub(crate) fn unix_timestamp_ms() -> Result<i64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::internal("the system clock is before the Unix epoch"))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| AppError::internal("the system timestamp is out of range"))
}

#[cfg(test)]
mod tests {
    use super::key_hint;

    #[test]
    fn key_hint_only_exposes_the_last_four_characters() {
        assert_eq!(key_hint("sk-example-123456"), "•••• 3456");
    }
}
