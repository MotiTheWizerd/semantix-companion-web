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

use crate::{
    app_error::AppError,
    inference::{api_provider_spec, API_PROVIDERS},
    secret_vault::SecretVault,
};

pub(crate) const CREDENTIALS_CHANGED_EVENT: &str = "credentials://changed";

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateProviderCredentialInput {
    credential_id: String,
    provider_id: String,
    label: String,
    api_key: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CredentialChangedEvent {
    Created {
        credential: CredentialMetadata,
    },
    Updated {
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

    fn update(&self, input: UpdateProviderCredentialInput) -> Result<CredentialMetadata, AppError> {
        let UpdateProviderCredentialInput {
            credential_id,
            provider_id,
            label,
            api_key,
        } = input;
        let current = self
            .repository
            .get(credential_id.trim())?
            .ok_or_else(|| AppError::validation("That saved API key no longer exists."))?;
        let provider = provider_by_id(provider_id.trim())
            .ok_or_else(|| AppError::validation("Choose a supported model provider."))?;

        if provider.id != current.metadata.provider_id
            && self.repository.is_used_by_model(&current.metadata.id)?
        {
            return Err(AppError::validation(
                "This API key is used by a configured model. Keep its provider or update the model first.",
            ));
        }

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

        let replacement_key = api_key
            .map(Zeroizing::new)
            .filter(|api_key| !api_key.trim().is_empty());
        if replacement_key
            .as_ref()
            .is_some_and(|api_key| api_key.trim().chars().count() < 8)
        {
            return Err(AppError::validation("Enter a complete provider API key."));
        }

        let mut updated = current.clone();
        updated.metadata.provider_id = provider.id;
        updated.metadata.label = label;
        updated.metadata.updated_at = unix_timestamp_ms()?;

        if let Some(api_key) = replacement_key {
            let api_key = api_key.trim();
            let previous_key = SecretVault::get(&current.secret_ref)?;
            updated.metadata.key_hint = key_hint(api_key);
            SecretVault::store(&current.secret_ref, api_key)?;
            if let Err(error) = self.repository.update(&updated) {
                let _ = SecretVault::store(&current.secret_ref, previous_key.as_str());
                return Err(error);
            }
        } else {
            self.repository.update(&updated)?;
        }

        Ok(updated.metadata)
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
    API_PROVIDERS
        .iter()
        .map(|provider| ProviderDefinition {
            id: provider.id.to_owned(),
            name: provider.name.to_owned(),
            key_placeholder: provider.key_placeholder.to_owned(),
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
pub(crate) async fn update_provider_credential(
    app: AppHandle,
    state: State<'_, CredentialState>,
    input: UpdateProviderCredentialInput,
) -> Result<CredentialMetadata, String> {
    let service = Arc::clone(&state.service);
    let credential = tauri::async_runtime::spawn_blocking(move || service.update(input))
        .await
        .map_err(|error| format!("Credential task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        CREDENTIALS_CHANGED_EVENT,
        CredentialChangedEvent::Updated {
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
    api_provider_spec(id).map(|provider| ProviderDefinition {
        id: provider.id.to_owned(),
        name: provider.name.to_owned(),
        key_placeholder: provider.key_placeholder.to_owned(),
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
    use super::{
        key_hint, list_known_model_providers, provider_by_id, CredentialChangedEvent,
        CredentialMetadata, UpdateProviderCredentialInput,
    };

    #[test]
    fn settings_only_offer_providers_the_runtime_can_execute() {
        let providers = list_known_model_providers();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["together", "openrouter"]
        );
        assert_eq!(
            provider_by_id("openrouter").map(|provider| provider.name),
            Some("OpenRouter".to_owned())
        );
        assert!(provider_by_id("unconnected-provider").is_none());
    }

    #[test]
    fn key_hint_only_exposes_the_last_four_characters() {
        assert_eq!(key_hint("sk-example-123456"), "•••• 3456");
    }

    #[test]
    fn credential_update_accepts_an_omitted_replacement_key() {
        let input: UpdateProviderCredentialInput = serde_json::from_value(serde_json::json!({
            "credentialId": "credential-123",
            "providerId": "openai",
            "label": "Primary"
        }))
        .expect("credential update should deserialize");

        assert_eq!(input.credential_id, "credential-123");
        assert_eq!(input.provider_id, "openai");
        assert_eq!(input.label, "Primary");
        assert!(input.api_key.is_none());
    }

    #[test]
    fn updated_credential_event_uses_the_camel_case_ipc_contract() {
        let event = serde_json::to_value(CredentialChangedEvent::Updated {
            credential: CredentialMetadata {
                id: "credential-123".to_owned(),
                provider_id: "openai".to_owned(),
                label: "Primary".to_owned(),
                key_hint: "•••• 1234".to_owned(),
                created_at: 1,
                updated_at: 2,
                last_used_at: None,
            },
        })
        .expect("updated credential event should serialize");

        assert_eq!(event["kind"], "updated");
        assert_eq!(event["credential"]["providerId"], "openai");
        assert!(event["credential"].get("provider_id").is_none());
    }
}
