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

use std::{collections::HashSet, path::Path, sync::Arc};

pub(crate) use repository::CompanionRepository;
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
const MAX_WORKSPACE_LABEL_LENGTH: usize = 80;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionWorkspace {
    pub(crate) id: String,
    /// The user's stable, human-facing name for this capability.
    pub(crate) label: String,
    /// Canonical absolute path. It is shown in Settings but never handed to a
    /// model; file tools see only `label` plus a path relative to this root.
    pub(crate) directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompanionWorkspaceInput {
    /// Present for an existing row. Unknown ids are ignored and reminted so a
    /// caller cannot move another companion's grant by guessing its id.
    #[serde(default)]
    id: Option<String>,
    label: String,
    directory: String,
}

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
    /// Named folders this companion's file tools may touch. Empty means the
    /// file tools are never offered to the model.
    pub(crate) workspaces: Vec<CompanionWorkspace>,
    /// WHICH Postgres holds this companion's memories — never what it is.
    /// `false` (the only value a public install ever has) is the Semantix
    /// organ; `true` is the machine-local canonical Muninn, unreachable from
    /// anywhere else. There is no setter: it is a hand-edit of the local
    /// database, on purpose. See schema 16.
    pub(crate) is_origin: bool,
    /// WHO this companion signs its carvings as — a UUID on Muninn's holy
    /// list, sent as `X-Agent-Id`. `None` leaves the author NULL, which the
    /// per-prompt recall reads as "an unidentified raven" and warns the reader
    /// off their own memory. Only meaningful alongside `is_origin`; the organ
    /// takes its identity from the bearer token. See schema 17.
    pub(crate) origin_agent_id: Option<String>,
}

/// What an origin companion is allowed to sign with. Absent = the companion is
/// not an origin at all, which is a different thing from an origin that has no
/// identity yet — the first goes to the organ, the second carves anonymously.
pub(crate) struct OriginIdentity {
    pub(crate) agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCompanionInput {
    name: Option<String>,
    model_preference: ModelPreference,
    #[serde(default)]
    workspaces: Vec<CompanionWorkspaceInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCompanionInput {
    companion_id: String,
    name: Option<String>,
    model_preference: ModelPreference,
    #[serde(default)]
    workspaces: Vec<CompanionWorkspaceInput>,
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
    /// Whether this id names a companion that actually lives here.
    ///
    /// The question `resolve` cannot answer, because it is built to always
    /// return SOMEONE. A caller that must not silently address the wrong
    /// companion asks this first.
    pub(crate) fn exists(&self, companion_id: &str) -> Result<bool, AppError> {
        Ok(self.repository.get(companion_id.trim())?.is_some())
    }

    /// Whether the companion behind this memory agent reads from the canonical
    /// Muninn rather than the Semantix organ, and who it signs as when it does.
    /// See `Companion::is_origin` and `Companion::origin_agent_id`.
    pub(crate) fn origin_identity(
        &self,
        memory_agent_name: &str,
    ) -> Result<Option<OriginIdentity>, AppError> {
        self.repository.origin_identity(memory_agent_name.trim())
    }

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
            workspaces,
        } = input;
        let name = normalise_name(name)?;
        let workspaces = normalise_workspaces(workspaces, &[])?;
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
            workspaces,
            // A companion made through the app always speaks to the organ.
            // Pointing one at the canonical Muninn is a hand-edit, never a
            // thing the UI can do — see schema 16.
            is_origin: false,
            origin_agent_id: None,
        };

        self.repository.insert(&companion)?;
        Ok(companion)
    }

    fn update(&self, input: UpdateCompanionInput) -> Result<Companion, AppError> {
        let UpdateCompanionInput {
            companion_id,
            name,
            model_preference,
            workspaces,
        } = input;
        let current = self.require(companion_id.trim())?;
        let name = normalise_name(name)?;
        let workspaces = normalise_workspaces(workspaces, &current.workspaces)?;
        self.preferences
            .validate_model_preference(&model_preference)?;
        let timestamp = unix_timestamp_ms()?;
        self.repository.update_details(
            &current.id,
            name.as_deref(),
            &model_preference,
            &workspaces,
            timestamp,
        )?;

        Ok(Companion {
            name,
            model_preference,
            workspaces,
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

/// Validate and canonicalise the complete grant set before a transaction ever
/// touches the database. Labels and roots are unique per companion so a tool
/// call can name exactly one capability without ambiguity.
fn normalise_workspaces(
    inputs: Vec<CompanionWorkspaceInput>,
    existing: &[CompanionWorkspace],
) -> Result<Vec<CompanionWorkspace>, AppError> {
    let existing_ids: HashSet<&str> = existing.iter().map(|item| item.id.as_str()).collect();
    let mut used_ids = HashSet::new();
    let mut labels = HashSet::new();
    let mut directories = HashSet::new();
    let mut workspaces = Vec::with_capacity(inputs.len());

    for input in inputs {
        let label = input.label.trim();
        if label.is_empty() {
            return Err(AppError::validation("Every workspace folder needs a name."));
        }
        if label.chars().count() > MAX_WORKSPACE_LABEL_LENGTH {
            return Err(AppError::validation(format!(
                "A workspace name must be {MAX_WORKSPACE_LABEL_LENGTH} characters or fewer."
            )));
        }
        if !labels.insert(label.to_lowercase()) {
            return Err(AppError::validation(
                "Workspace folder names must be unique for this companion.",
            ));
        }

        let directory = input.directory.trim();
        if directory.is_empty() {
            return Err(AppError::validation("Every workspace needs a folder."));
        }
        let canonical = std::fs::canonicalize(directory).map_err(|_| {
            AppError::validation("A workspace folder could not be found on this machine.")
        })?;
        if !canonical.is_dir() {
            return Err(AppError::validation(
                "A workspace must be a folder, not a file.",
            ));
        }
        let directory = canonical.into_os_string().into_string().map_err(|_| {
            AppError::validation("A workspace folder's path is not valid UTF-8.")
        })?;
        if !directories.insert(directory.clone()) {
            return Err(AppError::validation(
                "The same folder cannot be added to one companion twice.",
            ));
        }

        let requested_id = input.id.as_deref().map(str::trim).filter(|id| {
            !id.is_empty() && existing_ids.contains(*id) && !used_ids.contains(*id)
        });
        let id = requested_id
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        used_ids.insert(id.clone());
        workspaces.push(CompanionWorkspace {
            id,
            label: label.to_owned(),
            directory,
        });
    }

    Ok(workspaces)
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
    use std::{env, fs, path::{Path, PathBuf}};

    use uuid::Uuid;

    use super::{
        normalise_name, Companion, CompanionChangedEvent, CompanionRepository, CompanionResolver,
        CompanionService, CompanionWorkspace, CompanionWorkspaceInput, CreateCompanionInput,
        UpdateCompanionInput, MAX_NAME_LENGTH,
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

    /// The public product's guarantee, held by a test rather than by care: a
    /// build nobody has hand-edited routes every companion to the organ.
    #[test]
    fn no_companion_is_an_origin_companion_without_a_hand_edit() {
        let database = ScratchDatabase::new();
        let service = database.service();

        let built_in = service
            .repository
            .built_in()
            .expect("the built-in should load")
            .expect("a fresh install seeds one");
        assert!(!built_in.is_origin);

        let made = service
            .create(CreateCompanionInput {
                name: Some("Rook".to_owned()),
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
            })
            .expect("a companion should be creatable");
        assert!(!made.is_origin, "the app has no door that sets this");

        let resolver = database.resolver();
        assert!(resolver
            .origin_identity(&made.memory_agent_name)
            .expect("the lookup should run")
            .is_none());
    }

    /// An agent id the roster has never seen is an ORGAN id — the uuid every
    /// normal companion carries. It must never be read as a Muninn channel,
    /// or a plain install would try to address a brain it cannot reach.
    #[test]
    fn an_unknown_agent_name_routes_to_the_organ() {
        let database = ScratchDatabase::new();
        let resolver = database.resolver();

        for stranger in ["", "   ", "8f0d3c1e-0000-4000-8000-000000000000", "companion-nope"] {
            assert!(
                resolver.origin_identity(stranger).expect("the lookup should run").is_none(),
                "unknown agent {stranger:?} must fall through to the organ"
            );
        }
    }

    /// An origin companion with no identity still routes to Muninn — it just
    /// carves anonymously. Losing the name is bad; losing the memory is worse,
    /// so the missing id must never be read as "not an origin".
    #[test]
    fn an_origin_without_an_identity_is_still_an_origin() {
        let database = ScratchDatabase::new();
        let made = database
            .service()
            .create(CreateCompanionInput {
                name: Some("Arc".to_owned()),
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
            })
            .expect("a companion should be creatable");

        let connection =
            database::open_connection(&database.path).expect("the database should open");
        connection
            .execute("UPDATE companions SET is_origin = 1 WHERE id = ?1", [&made.id])
            .expect("the hand-edit should apply");

        let identity = database
            .resolver()
            .origin_identity(&made.memory_agent_name)
            .expect("the lookup should run")
            .expect("an origin with no id is still an origin");
        assert!(identity.agent_id.is_none(), "nothing was signed, so nothing is claimed");
    }

    /// The signature the carve rides on. A blank column must read as unsigned
    /// rather than as an empty header, which Muninn would reject outright.
    #[test]
    fn a_hand_signed_companion_carries_its_agent_id_and_blanks_do_not_count() {
        let database = ScratchDatabase::new();
        let made = database
            .service()
            .create(CreateCompanionInput {
                name: Some("Studio".to_owned()),
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
            })
            .expect("a companion should be creatable");
        let connection =
            database::open_connection(&database.path).expect("the database should open");

        for (written, expected) in [
            ("  ad629d1f-6665-4eb7-b272-8579f68fa0bb  ", Some("ad629d1f-6665-4eb7-b272-8579f68fa0bb")),
            ("   ", None),
        ] {
            connection
                .execute(
                    "UPDATE companions SET is_origin = 1, origin_agent_id = ?2 WHERE id = ?1",
                    rusqlite::params![&made.id, written],
                )
                .expect("the hand-edit should apply");

            let identity = database
                .resolver()
                .origin_identity(&made.memory_agent_name)
                .expect("the lookup should run")
                .expect("the flag is set");
            assert_eq!(identity.agent_id.as_deref(), expected, "written as {written:?}");
        }
    }

    /// Flipping the flag is a hand-edit of the local database, on purpose —
    /// this proves the read side honours it once someone does.
    #[test]
    fn a_hand_flipped_companion_is_recognised_as_an_origin_companion() {
        let database = ScratchDatabase::new();
        let made = database
            .service()
            .create(CreateCompanionInput {
                name: Some("Arc".to_owned()),
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
            })
            .expect("a companion should be creatable");

        let connection =
            database::open_connection(&database.path).expect("the database should open");
        connection
            .execute(
                "UPDATE companions SET is_origin = 1 WHERE id = ?1",
                [&made.id],
            )
            .expect("the hand-edit should apply");

        let resolver = database.resolver();
        assert!(resolver
            .origin_identity(&made.memory_agent_name)
            .expect("the lookup should run")
            .is_some());
        // and it stays scoped to that one companion
        let built_in = database
            .service()
            .repository
            .built_in()
            .expect("the built-in should load")
            .expect("a fresh install seeds one");
        assert!(resolver
            .origin_identity(&built_in.memory_agent_name)
            .expect("the lookup should run")
            .is_none());
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
                workspaces: Vec::new(),
            })
            .expect("the first companion should be created");
        let second = service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
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
                workspaces: Vec::new(),
            })
            .expect("the companion should be created");

        let renamed = service
            .update(UpdateCompanionInput {
                companion_id: created.id.clone(),
                name: Some("  Bjorn  ".to_owned()),
                model_preference: ModelPreference::Test,
                workspaces: Vec::new(),
            })
            .expect("the companion should rename");
        assert_eq!(renamed.name.as_deref(), Some("Bjorn"));
        assert_eq!(renamed.memory_agent_name, created.memory_agent_name);

        let cleared = service
            .update(UpdateCompanionInput {
                companion_id: created.id,
                name: Some(String::new()),
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
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
                workspaces: Vec::new(),
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
            "modelPreference": { "mode": "configured", "modelId": "model-123" },
            "workspaces": [{
                "label": "Code",
                "directory": "/tmp/code"
            }]
        }))
        .expect("create input should deserialize");
        assert_eq!(input.name.as_deref(), Some("Ragnar"));
        assert_eq!(
            input.model_preference,
            ModelPreference::Configured {
                model_id: "model-123".to_owned()
            }
        );
        assert_eq!(input.workspaces.len(), 1);
        assert_eq!(input.workspaces[0].label, "Code");
        assert!(input.workspaces[0].id.is_none());

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
            "modelPreference": { "mode": "test" },
            "workspaces": [{
                "id": "workspace-1",
                "label": "Notes",
                "directory": "/tmp/notes"
            }]
        }))
        .expect("update input should deserialize");
        assert_eq!(input.companion_id, "companion-built-in");
        assert!(input.name.is_none());
        assert_eq!(input.workspaces[0].id.as_deref(), Some("workspace-1"));
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
                workspaces: vec![CompanionWorkspace {
                    id: "workspace-1".to_owned(),
                    label: "Code".to_owned(),
                    directory: "/tmp/code".to_owned(),
                }],
                is_origin: false,
                origin_agent_id: None,
            },
        })
        .expect("created event should serialize");
        assert_eq!(created["kind"], "created");
        assert_eq!(created["companion"]["memoryAgentName"], "companion-1-memory");
        assert_eq!(created["companion"]["modelPreference"]["mode"], "inherit");
        assert_eq!(created["companion"]["isBuiltIn"], false);
        assert_eq!(created["companion"]["workspaces"][0]["label"], "Code");
        assert_eq!(
            created["companion"]["workspaces"][0]["directory"],
            "/tmp/code"
        );
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
                workspaces: Vec::new(),
            })
            .expect("the companion should be created");
        let resolved = resolver
            .resolve(Some(&added.id))
            .expect("an explicit pick should resolve");
        assert_eq!(resolved.id, added.id);
        assert_eq!(resolved.model_preference, ModelPreference::Test);
    }

    #[test]
    fn named_workspaces_are_canonical_ordered_and_revocable() {
        let database = ScratchDatabase::new();
        let service = database.service();
        let base = env::temp_dir().join(format!("companion-workspaces-{}", Uuid::new_v4()));
        let notes = base.join("notes");
        let code = base.join("code");
        fs::create_dir_all(&notes).expect("the notes workspace should be created");
        fs::create_dir_all(&code).expect("the code workspace should be created");

        let created = service
            .create(CreateCompanionInput {
                name: Some("Bjorn".to_owned()),
                model_preference: ModelPreference::Inherit,
                workspaces: vec![
                    CompanionWorkspaceInput {
                        id: None,
                        label: " Notes ".to_owned(),
                        directory: notes.to_string_lossy().into_owned(),
                    },
                    CompanionWorkspaceInput {
                        id: None,
                        label: "Code".to_owned(),
                        directory: code.to_string_lossy().into_owned(),
                    },
                ],
            })
            .expect("a companion with workspaces should be created");
        assert_eq!(created.workspaces.len(), 2);
        assert_eq!(created.workspaces[0].label, "Notes");
        assert_eq!(
            created.workspaces[0].directory,
            fs::canonicalize(&notes)
                .expect("the notes workspace should canonicalise")
                .to_string_lossy(),
            "the stored path is the canonical one"
        );

        // Order and ids survive a round-trip through chat's resolver.
        let resolved = database
            .resolver()
            .resolve(Some(&created.id))
            .expect("the companion should resolve");
        assert_eq!(resolved.workspaces[0].id, created.workspaces[0].id);
        assert_eq!(resolved.workspaces[1].label, "Code");

        // Reordering and renaming keep the existing capability ids.
        let reordered = service
            .update(UpdateCompanionInput {
                companion_id: created.id.clone(),
                name: created.name.clone(),
                model_preference: ModelPreference::Inherit,
                workspaces: vec![
                    CompanionWorkspaceInput {
                        id: Some(created.workspaces[1].id.clone()),
                        label: "Source".to_owned(),
                        directory: code.to_string_lossy().into_owned(),
                    },
                    CompanionWorkspaceInput {
                        id: Some(created.workspaces[0].id.clone()),
                        label: "Notes".to_owned(),
                        directory: notes.to_string_lossy().into_owned(),
                    },
                ],
            })
            .expect("the workspaces should reorder");
        assert_eq!(reordered.workspaces[0].id, created.workspaces[1].id);
        assert_eq!(reordered.workspaces[0].label, "Source");

        // An empty collection revokes every file capability atomically.
        let cleared = service
            .update(UpdateCompanionInput {
                companion_id: reordered.id,
                name: None,
                model_preference: ModelPreference::Inherit,
                workspaces: Vec::new(),
            })
            .expect("the workspaces should clear");
        assert!(cleared.workspaces.is_empty());

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_workspace_that_is_not_an_existing_folder_is_rejected() {
        let database = ScratchDatabase::new();
        let service = database.service();
        assert!(
            service
                .create(CreateCompanionInput {
                    name: None,
                    model_preference: ModelPreference::Inherit,
                    workspaces: vec![CompanionWorkspaceInput {
                        id: None,
                        label: "Missing".to_owned(),
                        directory: "/definitely/not/a/real/folder".to_owned(),
                    }],
                })
                .is_err(),
            "a missing folder must be rejected, not stored"
        );
    }

    #[test]
    fn workspace_names_and_directories_are_unique_per_companion() {
        let database = ScratchDatabase::new();
        let service = database.service();
        let directory =
            env::temp_dir().join(format!("companion-workspace-{}", Uuid::new_v4()));
        let other = directory.join("other");
        fs::create_dir_all(&other).expect("the scratch folders should be created");

        let input = |label: &str, path: &Path| CompanionWorkspaceInput {
            id: None,
            label: label.to_owned(),
            directory: path.to_string_lossy().into_owned(),
        };
        assert!(service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
                workspaces: vec![input("Code", &directory), input("code", &other)],
            })
            .is_err());
        assert!(service
            .create(CreateCompanionInput {
                name: None,
                model_preference: ModelPreference::Inherit,
                workspaces: vec![input("Code", &directory), input("Mirror", &directory)],
            })
            .is_err());

        let _ = fs::remove_dir_all(&directory);
    }
}
