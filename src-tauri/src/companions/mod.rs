//! Companions — who you talk to.
//!
//! A companion is a name (optional; it may sit unnamed), ONE private memory,
//! and the model it speaks with. The companion is the IDENTITY: a conversation
//! picks a companion, never a model, and the companion's voice answers.
//!
//! The memory is an agent on the organ's roster, addressed by
//! `memory_agent_name`: it is minted with the record, unique across companions,
//! and immutable, so a rename can never orphan what a companion remembers and
//! two companions can never read each other's piles.
//!
//! The agent itself is created lazily on the organ (`ensure_memory_agent`) the
//! first time a companion actually speaks — adding one here costs nothing
//! upstream.

mod repository;

use std::{path::Path, sync::Arc};

use repository::CompanionRepository;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{
    app_error::AppError,
    credentials::unix_timestamp_ms,
    preferences::{ModelPreference, PreferenceRepository},
};

pub(crate) const COMPANIONS_CHANGED_EVENT: &str = "companions://changed";

const MAX_NAME_LENGTH: usize = 80;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Companion {
    pub(crate) id: String,
    /// `None` = unnamed. The UI shows a placeholder; it is not an error state.
    pub(crate) name: Option<String>,
    /// This companion's private memory on the organ roster. Never reassigned.
    pub(crate) memory_agent_name: String,
    /// The voice this companion speaks with. `Inherit` follows the user's
    /// default model, so most companions never need an explicit choice.
    pub(crate) model_preference: ModelPreference,
    /// The seeded companion. It owns the memory the app carved before
    /// companions existed, and cannot be removed.
    pub(crate) is_built_in: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCompanionInput {
    name: Option<String>,
    model_preference: ModelPreference,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCompanionInput {
    companion_id: String,
    name: Option<String>,
    model_preference: ModelPreference,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CompanionChangedEvent {
    Created {
        companion: Companion,
    },
    Updated {
        companion: Companion,
    },
    Deleted {
        #[serde(rename = "companionId")]
        companion_id: String,
    },
}

pub(crate) struct CompanionState {
    service: Arc<CompanionService>,
}

/// The chat side's read-only door onto the roster. Chat never edits a
/// companion; it only asks who is answering and with what voice.
pub(crate) struct CompanionResolver {
    repository: CompanionRepository,
}

impl CompanionResolver {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            repository: CompanionRepository::open(database_path)?,
        })
    }

    /// Who answers this thread. A named companion wins; otherwise the built-in
    /// one — which is why deleting a companion leaves its old threads working
    /// rather than mute, and why a brand-new thread needs no pick to send.
    pub(crate) fn resolve(&self, companion_id: Option<&str>) -> Result<Companion, AppError> {
        if let Some(id) = companion_id.map(str::trim).filter(|id| !id.is_empty()) {
            if let Some(companion) = self.repository.get(id)? {
                return Ok(companion);
            }
        }
        self.repository.built_in()?.ok_or_else(|| {
            AppError::internal("the built-in companion is missing from the roster")
        })
    }
}

impl CompanionState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(CompanionService {
                repository: CompanionRepository::open(database_path)?,
                preferences: PreferenceRepository::open(database_path)?,
            }),
        })
    }
}

struct CompanionService {
    repository: CompanionRepository,
    preferences: PreferenceRepository,
}

impl CompanionService {
    fn list(&self) -> Result<Vec<Companion>, AppError> {
        self.repository.list()
    }

    fn create(&self, input: CreateCompanionInput) -> Result<Companion, AppError> {
        let CreateCompanionInput {
            name,
            model_preference,
        } = input;
        let name = normalise_name(name)?;
        self.preferences
            .validate_model_preference(&model_preference)?;
        let id = Uuid::new_v4().to_string();
        let timestamp = unix_timestamp_ms()?;
        let companion = Companion {
            memory_agent_name: format!("companion-{id}"),
            id,
            name,
            model_preference,
            is_built_in: false,
            created_at: timestamp,
            updated_at: timestamp,
        };

        self.repository.insert(&companion)?;
        Ok(companion)
    }

    fn update(&self, input: UpdateCompanionInput) -> Result<Companion, AppError> {
        let UpdateCompanionInput {
            companion_id,
            name,
            model_preference,
        } = input;
        let current = self.require(companion_id.trim())?;
        let name = normalise_name(name)?;
        self.preferences
            .validate_model_preference(&model_preference)?;
        let timestamp = unix_timestamp_ms()?;
        self.repository
            .update_details(&current.id, name.as_deref(), &model_preference, timestamp)?;

        Ok(Companion {
            name,
            model_preference,
            updated_at: timestamp,
            ..current
        })
    }

    fn delete(&self, id: &str) -> Result<(), AppError> {
        let companion = self.require(id)?;
        if companion.is_built_in {
            return Err(AppError::validation(
                "The built-in companion cannot be removed.",
            ));
        }
        self.repository.delete(&companion.id)
    }

    fn require(&self, id: &str) -> Result<Companion, AppError> {
        self.repository
            .get(id)?
            .ok_or_else(|| AppError::validation("That companion no longer exists."))
    }
}

/// Blank, whitespace, or absent all mean the same thing: unnamed.
fn normalise_name(name: Option<String>) -> Result<Option<String>, AppError> {
    let Some(name) = name else { return Ok(None) };
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > MAX_NAME_LENGTH {
        return Err(AppError::validation(format!(
            "A companion name must be {MAX_NAME_LENGTH} characters or fewer."
        )));
    }
    Ok(Some(name.to_owned()))
}

#[tauri::command]
pub(crate) async fn list_companions(
    state: State<'_, CompanionState>,
) -> Result<Vec<Companion>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.list())
        .await
        .map_err(|error| format!("Companion task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn create_companion(
    app: AppHandle,
    state: State<'_, CompanionState>,
    input: CreateCompanionInput,
) -> Result<Companion, String> {
    let service = Arc::clone(&state.service);
    let companion = tauri::async_runtime::spawn_blocking(move || service.create(input))
        .await
        .map_err(|error| format!("Companion task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        COMPANIONS_CHANGED_EVENT,
        CompanionChangedEvent::Created {
            companion: companion.clone(),
        },
    );

    Ok(companion)
}

#[tauri::command]
pub(crate) async fn update_companion(
    app: AppHandle,
    state: State<'_, CompanionState>,
    input: UpdateCompanionInput,
) -> Result<Companion, String> {
    let service = Arc::clone(&state.service);
    let companion = tauri::async_runtime::spawn_blocking(move || service.update(input))
        .await
        .map_err(|error| format!("Companion task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        COMPANIONS_CHANGED_EVENT,
        CompanionChangedEvent::Updated {
            companion: companion.clone(),
        },
    );

    Ok(companion)
}

#[tauri::command]
pub(crate) async fn delete_companion(
    app: AppHandle,
    state: State<'_, CompanionState>,
    companion_id: String,
) -> Result<(), String> {
    let service = Arc::clone(&state.service);
    let id = companion_id.clone();
    tauri::async_runtime::spawn_blocking(move || service.delete(&id))
        .await
        .map_err(|error| format!("Companion task failed: {error}"))?
        .map_err(String::from)?;

    let _ = app.emit(
        COMPANIONS_CHANGED_EVENT,
        CompanionChangedEvent::Deleted { companion_id },
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use uuid::Uuid;

    use super::{
        normalise_name, Companion, CompanionChangedEvent, CompanionRepository, CompanionResolver,
        CompanionService, CreateCompanionInput, UpdateCompanionInput, MAX_NAME_LENGTH,
    };
    use crate::{database, preferences::{ModelPreference, PreferenceRepository}};

    /// A throwaway on-disk database — the repository opens by path, and the
    /// crate carries no temp-file dependency worth adding for this.
    struct ScratchDatabase {
        path: PathBuf,
    }

    impl ScratchDatabase {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("companion-test-{}.db", Uuid::new_v4()));
            database::initialise(&path).expect("the scratch database should migrate");
            Self { path }
        }

        fn service(&self) -> CompanionService {
            CompanionService {
                repository: CompanionRepository::open(&self.path)
                    .expect("the repository should open"),
                preferences: PreferenceRepository::open(&self.path)
                    .expect("the preference repository should open"),
            }
        }

        fn resolver(&self) -> CompanionResolver {
            CompanionResolver::open(&self.path).expect("the resolver should open")
        }
    }

    impl Drop for ScratchDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            for suffix in ["-wal", "-shm"] {
                let mut sidecar = self.path.clone().into_os_string();
                sidecar.push(suffix);
                let _ = fs::remove_file(PathBuf::from(sidecar));
            }
        }
    }

    #[test]
    fn a_fresh_install_has_exactly_one_unnamed_built_in_companion() {
        let database = ScratchDatabase::new();
        let companions = database.service().list().expect("the roster should list");

        assert_eq!(companions.len(), 1);
        assert!(companions[0].is_built_in);
        assert_eq!(companions[0].name, None);
        assert_eq!(companions[0].memory_agent_name, "companion");
    }

    #[test]
    fn each_added_companion_gets_its_own_private_memory() {
        let database = ScratchDatabase::new();
        let service = database.service();

        let first = service
            .create(CreateCompanionInput {
                name: Some("Ragnar".to_owned()),
                model_preference: ModelPreference::Test,
            })
            .expect("the first companion should be created");
        let second = service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
            })
            .expect("an unnamed companion should be created");

        assert_eq!(first.name.as_deref(), Some("Ragnar"));
        assert_eq!(second.name, None);
        assert_ne!(first.memory_agent_name, second.memory_agent_name);
        assert_ne!(first.memory_agent_name, "companion");

        let roster = service.list().expect("the roster should list");
        assert_eq!(roster.len(), 3);
        assert!(roster[0].is_built_in, "the built-in companion leads");
    }

    #[test]
    fn renaming_a_companion_leaves_its_memory_where_it_is() {
        let database = ScratchDatabase::new();
        let service = database.service();
        let created = service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
            })
            .expect("the companion should be created");

        let renamed = service
            .update(UpdateCompanionInput {
                companion_id: created.id.clone(),
                name: Some("  Bjorn  ".to_owned()),
                model_preference: ModelPreference::Test,
            })
            .expect("the companion should rename");
        assert_eq!(renamed.name.as_deref(), Some("Bjorn"));
        assert_eq!(renamed.memory_agent_name, created.memory_agent_name);

        let cleared = service
            .update(UpdateCompanionInput {
                companion_id: created.id,
                name: Some(String::new()),
                model_preference: ModelPreference::Inherit,
            })
            .expect("the name should clear back to unnamed");
        assert_eq!(cleared.name, None);
        assert_eq!(cleared.memory_agent_name, created.memory_agent_name);
    }

    #[test]
    fn the_built_in_companion_cannot_be_deleted_but_the_others_can() {
        let database = ScratchDatabase::new();
        let service = database.service();
        let added = service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
            })
            .expect("the companion should be created");

        assert!(
            service.delete("companion-built-in").is_err(),
            "the built-in companion must survive a delete"
        );
        service.delete(&added.id).expect("an added one should go");
        assert!(
            service.delete(&added.id).is_err(),
            "deleting twice should report the companion is gone"
        );

        let roster = service.list().expect("the roster should list");
        assert_eq!(roster.len(), 1);
        assert!(roster[0].is_built_in);
    }

    #[test]
    fn a_blank_name_is_stored_as_unnamed() {
        assert_eq!(normalise_name(None).expect("absent name is valid"), None);
        assert_eq!(
            normalise_name(Some("   ".to_owned())).expect("blank name is valid"),
            None
        );
        assert_eq!(
            normalise_name(Some("  Ragnar  ".to_owned())).expect("name is valid"),
            Some("Ragnar".to_owned())
        );
    }

    #[test]
    fn an_overlong_name_is_rejected() {
        let name = "a".repeat(MAX_NAME_LENGTH + 1);
        assert!(normalise_name(Some(name)).is_err());
    }

    #[test]
    fn create_input_accepts_the_camel_case_ipc_contract() {
        let input: CreateCompanionInput = serde_json::from_value(serde_json::json!({
            "name": "Ragnar",
            "modelPreference": { "mode": "configured", "modelId": "model-123" }
        }))
        .expect("create input should deserialize");
        assert_eq!(input.name.as_deref(), Some("Ragnar"));
        assert_eq!(
            input.model_preference,
            ModelPreference::Configured {
                model_id: "model-123".to_owned()
            }
        );

        let unnamed: CreateCompanionInput = serde_json::from_value(serde_json::json!({
            "modelPreference": { "mode": "inherit" }
        }))
        .expect("an omitted name should deserialize");
        assert!(unnamed.name.is_none());
    }

    #[test]
    fn update_input_accepts_the_camel_case_ipc_contract() {
        let input: UpdateCompanionInput = serde_json::from_value(serde_json::json!({
            "companionId": "companion-built-in",
            "name": null,
            "modelPreference": { "mode": "test" }
        }))
        .expect("update input should deserialize");
        assert_eq!(input.companion_id, "companion-built-in");
        assert!(input.name.is_none());
    }

    #[test]
    fn companion_events_use_the_camel_case_ipc_contract() {
        let created = serde_json::to_value(CompanionChangedEvent::Created {
            companion: Companion {
                id: "companion-1".to_owned(),
                name: None,
                memory_agent_name: "companion-1-memory".to_owned(),
                model_preference: ModelPreference::Inherit,
                is_built_in: false,
                created_at: 1,
                updated_at: 1,
            },
        })
        .expect("created event should serialize");
        assert_eq!(created["kind"], "created");
        assert_eq!(created["companion"]["memoryAgentName"], "companion-1-memory");
        assert_eq!(created["companion"]["modelPreference"]["mode"], "inherit");
        assert_eq!(created["companion"]["isBuiltIn"], false);
        assert!(created["companion"].get("memory_agent_name").is_none());

        let deleted = serde_json::to_value(CompanionChangedEvent::Deleted {
            companion_id: "companion-1".to_owned(),
        })
        .expect("deleted event should serialize");
        assert_eq!(deleted["kind"], "deleted");
        assert_eq!(deleted["companionId"], "companion-1");
        assert!(deleted.get("companion_id").is_none());
    }

    #[test]
    fn a_thread_with_no_companion_of_its_own_falls_back_to_the_built_in_one() {
        let database = ScratchDatabase::new();
        let resolver = database.resolver();

        assert!(
            resolver.resolve(None).expect("a bare thread should resolve").is_built_in,
            "no pick means the built-in companion answers"
        );
        assert!(
            resolver
                .resolve(Some("  "))
                .expect("a blank pick should resolve")
                .is_built_in
        );
        assert!(
            resolver
                .resolve(Some("a-companion-that-was-deleted"))
                .expect("a stale pick should resolve")
                .is_built_in,
            "a deleted companion leaves its threads working, not mute"
        );

        let added = database
            .service()
            .create(CreateCompanionInput {
                name: Some("Bjorn".to_owned()),
                model_preference: ModelPreference::Test,
            })
            .expect("the companion should be created");
        let resolved = resolver
            .resolve(Some(&added.id))
            .expect("an explicit pick should resolve");
        assert_eq!(resolved.id, added.id);
        assert_eq!(resolved.model_preference, ModelPreference::Test);
    }
}
