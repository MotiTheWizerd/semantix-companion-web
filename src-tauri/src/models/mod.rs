mod repository;

use std::{path::Path, sync::Arc};

use repository::ModelRepository;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    app_error::AppError,
    credentials::{key_hint, provider_by_id, repository::CredentialRepository, unix_timestamp_ms},
    secret_vault::SecretVault,
};

pub(crate) const MODELS_CHANGED_EVENT: &str = "models://changed";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfiguredModel {
    pub(crate) id: String,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) display_name: String,
    pub(crate) credential_id: Option<String>,
    pub(crate) credential_kind: String,
    pub(crate) credential_label: String,
    pub(crate) key_hint: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ModelRecord {
    pub(crate) model: ConfiguredModel,
    pub(crate) secret_ref: Option<String>,
    pub(crate) manual_key_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateConfiguredModelInput {
    provider_id: String,
    model_id: String,
    display_name: String,
    credential: ModelCredentialInput,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateConfiguredModelInput {
    model_id: String,
    provider_id: String,
    provider_model_id: String,
    display_name: String,
    credential: ModelCredentialInput,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ModelCredentialInput {
    Saved {
        #[serde(rename = "credentialId")]
        credential_id: String,
    },
    Manual {
        #[serde(rename = "apiKey")]
        api_key: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ModelChangedEvent {
    Created {
        model: ConfiguredModel,
    },
    Updated {
        model: ConfiguredModel,
    },
    Deleted {
        #[serde(rename = "modelId")]
        model_id: String,
    },
}

pub(crate) struct ModelState {
    service: Arc<ModelService>,
}

pub(crate) struct ResolvedConfiguredModel {
    pub(crate) configured_model_id: String,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) api_key: Zeroizing<String>,
}

pub(crate) struct ModelResolver {
    repository: ModelRepository,
    credentials: CredentialRepository,
}

impl ModelResolver {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            repository: ModelRepository::open(database_path)?,
            credentials: CredentialRepository::open(database_path)?,
        })
    }

    pub(crate) fn resolve(&self, id: &str) -> Result<ResolvedConfiguredModel, AppError> {
        let record = self
            .repository
            .get(id)?
            .ok_or_else(|| AppError::validation("That configured model no longer exists."))?;
        let secret_ref = if let Some(secret_ref) = record.secret_ref {
            secret_ref
        } else {
            let credential_id = record.model.credential_id.as_deref().ok_or_else(|| {
                AppError::internal("the configured model has no credential reference")
            })?;
            let credential = self.credentials.get(credential_id)?.ok_or_else(|| {
                AppError::validation("The saved API key for this model no longer exists.")
            })?;
            if credential.metadata.provider_id != record.model.provider_id {
                return Err(AppError::internal(
                    "the model and saved credential providers do not match",
                ));
            }
            credential.secret_ref
        };

        Ok(ResolvedConfiguredModel {
            configured_model_id: record.model.id,
            provider_id: record.model.provider_id,
            model_id: record.model.model_id,
            api_key: SecretVault::get(&secret_ref)?,
        })
    }
}

impl ModelState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(ModelService {
                repository: ModelRepository::open(database_path)?,
                credentials: CredentialRepository::open(database_path)?,
            }),
        })
    }
}

struct ModelService {
    repository: ModelRepository,
    credentials: CredentialRepository,
}

impl ModelService {
    fn list(&self) -> Result<Vec<ConfiguredModel>, AppError> {
        self.repository.list()
    }

    fn create(&self, input: CreateConfiguredModelInput) -> Result<ConfiguredModel, AppError> {
        let CreateConfiguredModelInput {
            provider_id,
            model_id,
            display_name,
            credential,
        } = input;
        let provider = provider_by_id(provider_id.trim())
            .ok_or_else(|| AppError::validation("Choose a supported model provider."))?;
        let model_id = model_id.trim().to_owned();
        if model_id.is_empty() || model_id.chars().count() > 200 {
            return Err(AppError::validation(
                "Enter a model ID between 1 and 200 characters.",
            ));
        }

        let display_name = if display_name.trim().is_empty() {
            model_id.clone()
        } else {
            display_name.trim().to_owned()
        };
        if display_name.chars().count() > 100 {
            return Err(AppError::validation(
                "The model name must be 100 characters or fewer.",
            ));
        }

        let id = Uuid::new_v4().to_string();
        let timestamp = unix_timestamp_ms()?;
        let mut secret_ref = None;
        let mut manual_key_hint = None;

        let (credential_id, credential_kind, credential_label, resolved_key_hint) =
            match credential {
                ModelCredentialInput::Saved { credential_id } => {
                    let credential = self
                        .credentials
                        .get(credential_id.trim())?
                        .ok_or_else(|| AppError::validation("Choose an existing saved API key."))?;
                    if credential.metadata.provider_id != provider.id {
                        return Err(AppError::validation(
                            "The saved API key must belong to the selected provider.",
                        ));
                    }

                    (
                        Some(credential.metadata.id),
                        "saved".to_owned(),
                        credential.metadata.label,
                        credential.metadata.key_hint,
                    )
                }
                ModelCredentialInput::Manual { api_key } => {
                    let api_key = Zeroizing::new(api_key.ok_or_else(|| {
                        AppError::validation("Enter a complete provider API key.")
                    })?);
                    let api_key = api_key.trim();
                    if api_key.chars().count() < 8 {
                        return Err(AppError::validation("Enter a complete provider API key."));
                    }

                    let model_secret_ref = format!("model-api-key:{id}");
                    let hint = key_hint(api_key);
                    SecretVault::store(&model_secret_ref, api_key)?;
                    secret_ref = Some(model_secret_ref);
                    manual_key_hint = Some(hint.clone());

                    (None, "manual".to_owned(), "Manual key".to_owned(), hint)
                }
            };

        let model = ConfiguredModel {
            id,
            provider_id: provider.id,
            model_id,
            display_name,
            credential_id,
            credential_kind,
            credential_label,
            key_hint: resolved_key_hint,
            created_at: timestamp,
            updated_at: timestamp,
        };
        let record = ModelRecord {
            model: model.clone(),
            secret_ref: secret_ref.clone(),
            manual_key_hint,
        };

        if let Err(error) = self.repository.insert(&record) {
            if let Some(secret_ref) = secret_ref {
                let _ = SecretVault::delete(&secret_ref);
            }
            return Err(error);
        }

        Ok(model)
    }

    fn update(&self, input: UpdateConfiguredModelInput) -> Result<ConfiguredModel, AppError> {
        let UpdateConfiguredModelInput {
            model_id,
            provider_id,
            provider_model_id,
            display_name,
            credential,
        } = input;
        let current = self
            .repository
            .get(model_id.trim())?
            .ok_or_else(|| AppError::validation("That configured model no longer exists."))?;
        let provider = provider_by_id(provider_id.trim())
            .ok_or_else(|| AppError::validation("Choose a supported model provider."))?;
        let provider_model_id = provider_model_id.trim().to_owned();
        if provider_model_id.is_empty() || provider_model_id.chars().count() > 200 {
            return Err(AppError::validation(
                "Enter a model ID between 1 and 200 characters.",
            ));
        }

        let display_name = if display_name.trim().is_empty() {
            provider_model_id.clone()
        } else {
            display_name.trim().to_owned()
        };
        if display_name.chars().count() > 100 {
            return Err(AppError::validation(
                "The model name must be 100 characters or fewer.",
            ));
        }

        let timestamp = unix_timestamp_ms()?;
        match credential {
            ModelCredentialInput::Saved { credential_id } => {
                let credential = self
                    .credentials
                    .get(credential_id.trim())?
                    .ok_or_else(|| AppError::validation("Choose an existing saved API key."))?;
                if credential.metadata.provider_id != provider.id {
                    return Err(AppError::validation(
                        "The saved API key must belong to the selected provider.",
                    ));
                }

                let model = ConfiguredModel {
                    id: current.model.id.clone(),
                    provider_id: provider.id,
                    model_id: provider_model_id,
                    display_name,
                    credential_id: Some(credential.metadata.id),
                    credential_kind: "saved".to_owned(),
                    credential_label: credential.metadata.label,
                    key_hint: credential.metadata.key_hint,
                    created_at: current.model.created_at,
                    updated_at: timestamp,
                };
                let updated = ModelRecord {
                    model: model.clone(),
                    secret_ref: None,
                    manual_key_hint: None,
                };
                self.repository.update(&updated)?;

                if let Some(secret_ref) = &current.secret_ref {
                    if let Err(error) = SecretVault::delete(secret_ref) {
                        let _ = self.repository.update(&current);
                        return Err(error);
                    }
                }

                Ok(model)
            }
            ModelCredentialInput::Manual { api_key } => {
                let replacement_key = api_key
                    .map(Zeroizing::new)
                    .filter(|api_key| !api_key.trim().is_empty());
                if replacement_key
                    .as_ref()
                    .is_some_and(|api_key| api_key.trim().chars().count() < 8)
                {
                    return Err(AppError::validation("Enter a complete provider API key."));
                }

                if replacement_key.is_none() && current.secret_ref.is_none() {
                    return Err(AppError::validation(
                        "Enter an API key when switching this model to a manual key.",
                    ));
                }

                let secret_ref = current
                    .secret_ref
                    .clone()
                    .unwrap_or_else(|| format!("model-api-key:{}", current.model.id));
                let previous_key = if replacement_key.is_some() && current.secret_ref.is_some() {
                    Some(SecretVault::get(&secret_ref)?)
                } else {
                    None
                };
                let resolved_key_hint = replacement_key
                    .as_ref()
                    .map(|api_key| key_hint(api_key.trim()))
                    .or_else(|| current.manual_key_hint.clone())
                    .ok_or_else(|| AppError::internal("the manual model key hint is missing"))?;

                if let Some(api_key) = &replacement_key {
                    SecretVault::store(&secret_ref, api_key.trim())?;
                }

                let model = ConfiguredModel {
                    id: current.model.id.clone(),
                    provider_id: provider.id,
                    model_id: provider_model_id,
                    display_name,
                    credential_id: None,
                    credential_kind: "manual".to_owned(),
                    credential_label: "Manual key".to_owned(),
                    key_hint: resolved_key_hint.clone(),
                    created_at: current.model.created_at,
                    updated_at: timestamp,
                };
                let updated = ModelRecord {
                    model: model.clone(),
                    secret_ref: Some(secret_ref.clone()),
                    manual_key_hint: Some(resolved_key_hint),
                };

                if let Err(error) = self.repository.update(&updated) {
                    if replacement_key.is_some() {
                        if let Some(previous_key) = previous_key {
                            let _ = SecretVault::store(&secret_ref, previous_key.as_str());
                        } else {
                            let _ = SecretVault::delete(&secret_ref);
                        }
                    }
                    return Err(error);
                }

                Ok(model)
            }
        }
    }

    fn delete(&self, id: &str) -> Result<(), AppError> {
        let record = self
            .repository
            .get(id)?
            .ok_or_else(|| AppError::validation("That configured model no longer exists."))?;
        self.repository.delete(id)?;

        if let Some(secret_ref) = &record.secret_ref {
            if let Err(error) = SecretVault::delete(secret_ref) {
                let _ = self.repository.insert(&record);
                return Err(error);
            }
        }

        Ok(())
    }
}

#[tauri::command]
pub(crate) async fn list_configured_models(
    state: State<'_, ModelState>,
) -> Result<Vec<ConfiguredModel>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.list())
        .await
        .map_err(|error| format!("Model task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn create_configured_model(
    app: AppHandle,
    state: State<'_, ModelState>,
    input: CreateConfiguredModelInput,
) -> Result<ConfiguredModel, String> {
    let service = Arc::clone(&state.service);
    let model = tauri::async_runtime::spawn_blocking(move || service.create(input))
        .await
        .map_err(|error| format!("Model task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        MODELS_CHANGED_EVENT,
        ModelChangedEvent::Created {
            model: model.clone(),
        },
    );

    Ok(model)
}

#[tauri::command]
pub(crate) async fn update_configured_model(
    app: AppHandle,
    state: State<'_, ModelState>,
    input: UpdateConfiguredModelInput,
) -> Result<ConfiguredModel, String> {
    let service = Arc::clone(&state.service);
    let model = tauri::async_runtime::spawn_blocking(move || service.update(input))
        .await
        .map_err(|error| format!("Model task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        MODELS_CHANGED_EVENT,
        ModelChangedEvent::Updated {
            model: model.clone(),
        },
    );

    Ok(model)
}

#[tauri::command]
pub(crate) async fn delete_configured_model(
    app: AppHandle,
    state: State<'_, ModelState>,
    model_id: String,
) -> Result<(), String> {
    let service = Arc::clone(&state.service);
    let id = model_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.delete(&id))
        .await
        .map_err(|error| format!("Model task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        MODELS_CHANGED_EVENT,
        ModelChangedEvent::Deleted { model_id },
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ConfiguredModel, CreateConfiguredModelInput, ModelChangedEvent, ModelCredentialInput,
        UpdateConfiguredModelInput,
    };

    #[test]
    fn saved_credential_input_accepts_the_camel_case_ipc_contract() {
        let input: CreateConfiguredModelInput = serde_json::from_value(serde_json::json!({
            "providerId": "openai",
            "modelId": "gpt-test",
            "displayName": "Test model",
            "credential": {
                "kind": "saved",
                "credentialId": "credential-123"
            }
        }))
        .expect("saved credential input should deserialize");

        match input.credential {
            ModelCredentialInput::Saved { credential_id } => {
                assert_eq!(credential_id, "credential-123");
            }
            ModelCredentialInput::Manual { .. } => panic!("expected a saved credential"),
        }
    }

    #[test]
    fn manual_credential_input_accepts_the_camel_case_ipc_contract() {
        let input: CreateConfiguredModelInput = serde_json::from_value(serde_json::json!({
            "providerId": "openai",
            "modelId": "gpt-test",
            "displayName": "Test model",
            "credential": {
                "kind": "manual",
                "apiKey": "sk-example-1234"
            }
        }))
        .expect("manual credential input should deserialize");

        match input.credential {
            ModelCredentialInput::Manual { api_key } => {
                assert_eq!(api_key.as_deref(), Some("sk-example-1234"));
            }
            ModelCredentialInput::Saved { .. } => panic!("expected a manual credential"),
        }
    }

    #[test]
    fn deleted_model_event_uses_the_camel_case_ipc_contract() {
        let event = serde_json::to_value(ModelChangedEvent::Deleted {
            model_id: "model-123".to_owned(),
        })
        .expect("deleted model event should serialize");

        assert_eq!(event["kind"], "deleted");
        assert_eq!(event["modelId"], "model-123");
        assert!(event.get("model_id").is_none());
    }

    #[test]
    fn model_update_accepts_an_omitted_existing_manual_key() {
        let input: UpdateConfiguredModelInput = serde_json::from_value(serde_json::json!({
            "modelId": "configured-model-123",
            "providerId": "openai",
            "providerModelId": "gpt-test",
            "displayName": "Test model",
            "credential": {
                "kind": "manual"
            }
        }))
        .expect("model update should deserialize");

        assert_eq!(input.model_id, "configured-model-123");
        assert_eq!(input.provider_model_id, "gpt-test");
        match input.credential {
            ModelCredentialInput::Manual { api_key } => assert!(api_key.is_none()),
            ModelCredentialInput::Saved { .. } => panic!("expected a manual credential"),
        }
    }

    #[test]
    fn updated_model_event_uses_the_camel_case_ipc_contract() {
        let event = serde_json::to_value(ModelChangedEvent::Updated {
            model: ConfiguredModel {
                id: "model-123".to_owned(),
                provider_id: "openai".to_owned(),
                model_id: "gpt-test".to_owned(),
                display_name: "Test model".to_owned(),
                credential_id: None,
                credential_kind: "manual".to_owned(),
                credential_label: "Manual key".to_owned(),
                key_hint: "•••• 1234".to_owned(),
                created_at: 1,
                updated_at: 2,
            },
        })
        .expect("updated model event should serialize");

        assert_eq!(event["kind"], "updated");
        assert_eq!(event["model"]["providerId"], "openai");
        assert!(event["model"].get("provider_id").is_none());
    }
}
