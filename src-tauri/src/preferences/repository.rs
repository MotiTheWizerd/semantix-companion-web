use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};

use super::{ModelPreference, UserPreferences};
use crate::{app_error::AppError, database};

/// The voice a preference resolves to once 'inherit' has been followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedVoice {
    TestStream,
    /// A configured (OpenAI-compatible) model, by configured_models id.
    Configured(String),
    /// A Claude Code model, by SDK alias ("opus", "sonnet", …).
    ClaudeCode(String),
}

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
                // The picked companion is read THROUGH the roster: a pick that
                // names a companion since deleted comes back NULL, so every
                // reader sees "unset" instead of an id that points at nobody.
                "SELECT p.default_model_mode, p.default_model_id, p.display_name,
                        (SELECT c.id FROM companions c WHERE c.id = p.default_companion_id),
                        p.updated_at
                 FROM user_preferences p
                 WHERE p.id = 1",
                [],
                |row| {
                    let mode: String = row.get(0)?;
                    let model_id: Option<String> = row.get(1)?;
                    Ok(UserPreferences {
                        default_model: ModelPreference::from_storage(&mode, model_id),
                        display_name: row.get(2)?,
                        default_companion_id: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .map_err(AppError::database)
    }

    /// A PATCH: `None` for a field means "leave it as it is". Writing each
    /// column conditionally in ONE statement keeps the read-modify-write inside
    /// SQLite instead of round-tripping through the caller, so two settings
    /// screens open at once cannot overwrite each other's untouched fields.
    pub(crate) fn update_user_preferences(
        &self,
        default_model: Option<&ModelPreference>,
        display_name: Option<&str>,
        default_companion_id: Option<&str>,
        updated_at: i64,
    ) -> Result<UserPreferences, AppError> {
        if let Some(default_model) = default_model {
            if matches!(default_model, ModelPreference::Inherit) {
                return Err(AppError::validation(
                    "The user default model cannot inherit from another preference.",
                ));
            }
            self.validate_model_preference(default_model)?;
        }
        let name = display_name.map(normalize_display_name).transpose()?;
        // Blank clears, like the name; anything else must name a companion
        // that exists right now — a pick is made from the roster, never typed.
        let companion = match default_companion_id.map(str::trim) {
            None => None,
            Some("") => Some(None),
            Some(id) => {
                self.require_companion(id)?;
                Some(Some(id.to_owned()))
            }
        };
        let (mode, model_id) = match default_model {
            Some(preference) => {
                let (mode, model_id) = preference.storage_parts();
                (Some(mode), model_id)
            }
            None => (None, None),
        };

        // ⚑ SCOPED: the guard must be dropped before the read-back below. The
        // connection is behind a std Mutex, which is NOT reentrant — holding it
        // across a call that locks it again deadlocks the thread outright, and
        // `cargo check` is perfectly happy with it.
        {
            let connection = self.connection()?;
            connection
                .execute(
                    // COALESCE would refuse to write a NULL, and clearing the
                    // name is a legitimate save — so the guard is a separate
                    // "did the caller send this field" flag rather than the
                    // value itself.
                    "UPDATE user_preferences
                     SET default_model_mode   = CASE WHEN ?1 THEN ?2 ELSE default_model_mode END,
                         default_model_id     = CASE WHEN ?1 THEN ?3 ELSE default_model_id END,
                         display_name         = CASE WHEN ?4 THEN ?5 ELSE display_name END,
                         default_companion_id = CASE WHEN ?6 THEN ?7 ELSE default_companion_id END,
                         updated_at = ?8
                     WHERE id = 1",
                    params![
                        mode.is_some(),
                        mode,
                        model_id,
                        display_name.is_some(),
                        name.flatten(),
                        companion.is_some(),
                        companion.flatten(),
                        updated_at
                    ],
                )
                .map_err(AppError::database)?;
        }

        // Read back rather than assembling the answer from the inputs — the
        // untouched half of a patch is only knowable from the row.
        self.get_user_preferences()
    }

    fn require_companion(&self, companion_id: &str) -> Result<(), AppError> {
        let connection = self.connection()?;
        let exists = connection
            .query_row(
                "SELECT id FROM companions WHERE id = ?1",
                [companion_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(AppError::database)?
            .is_some();
        if !exists {
            return Err(AppError::validation("That companion no longer exists."));
        }
        Ok(())
    }

    pub(crate) fn validate_model_preference(
        &self,
        preference: &ModelPreference,
    ) -> Result<(), AppError> {
        // A Claude Code pick names an SDK model, not a configured_models row —
        // it only needs to name one at all.
        if let ModelPreference::ClaudeCode { model_id } = preference {
            if model_id.trim().is_empty() {
                return Err(AppError::validation("Choose a Claude Code model."));
            }
            return Ok(());
        }
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

    /// A preference resolved to the voice that will actually answer, with
    /// inherit already followed to the user default.
    pub(crate) fn resolve_voice(
        &self,
        preference: &ModelPreference,
    ) -> Result<ResolvedVoice, AppError> {
        let effective = if matches!(preference, ModelPreference::Inherit) {
            self.get_user_preferences()?.default_model
        } else {
            preference.clone()
        };
        self.validate_model_preference(&effective)?;
        Ok(match effective {
            ModelPreference::Configured { model_id } => ResolvedVoice::Configured(model_id),
            ModelPreference::ClaudeCode { model_id } => ResolvedVoice::ClaudeCode(model_id),
            ModelPreference::Inherit | ModelPreference::Test => ResolvedVoice::TestStream,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}

/// The longest a name may be. Generous enough for a full name in any script,
/// short enough that the sidebar's identity line cannot be turned into a
/// paragraph. Counted in CHARACTERS — a byte cap would cut a Hebrew or emoji
/// name at a third of the length it lets an ASCII one have.
const DISPLAY_NAME_MAX_CHARS: usize = 60;

/// Blank in, NULL out: a name typed and then erased means "I have no name set",
/// which must read the same as never having set one — otherwise every fallback
/// downstream needs to check for the empty string as well as NULL.
fn normalize_display_name(name: &str) -> Result<Option<String>, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > DISPLAY_NAME_MAX_CHARS {
        return Err(AppError::validation(format!(
            "A name can be at most {DISPLAY_NAME_MAX_CHARS} characters."
        )));
    }
    Ok(Some(name.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
    };

    use uuid::Uuid;

    use super::{normalize_display_name, PreferenceRepository, DISPLAY_NAME_MAX_CHARS};
    use crate::{database, preferences::ModelPreference};

    /// A throwaway on-disk database — the repository opens by path, and the
    /// crate carries no temp-file dependency worth adding for this.
    struct ScratchDatabase {
        path: PathBuf,
    }

    impl ScratchDatabase {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("preference-test-{}.db", Uuid::new_v4()));
            database::initialise(&path).expect("the scratch database should migrate");
            Self { path }
        }

        fn repository(&self) -> PreferenceRepository {
            PreferenceRepository::open(&self.path).expect("the repository should open")
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
    fn a_fresh_install_has_no_name() {
        let database = ScratchDatabase::new();
        let preferences = database
            .repository()
            .get_user_preferences()
            .expect("defaults should be readable");
        assert_eq!(preferences.display_name, None);
    }

    #[test]
    fn naming_yourself_leaves_the_default_model_alone() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        repository
            .update_user_preferences(Some(&ModelPreference::Test), None, None, 10)
            .expect("the model should save");

        let named = repository
            .update_user_preferences(None, Some("  Moti  "), None, 20)
            .expect("the name should save");

        assert_eq!(named.display_name.as_deref(), Some("Moti"));
        assert_eq!(named.default_model, ModelPreference::Test);
    }

    #[test]
    fn changing_the_model_leaves_the_name_alone() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        repository
            .update_user_preferences(None, Some("Moti"), None, 10)
            .expect("the name should save");

        let remodelled = repository
            .update_user_preferences(Some(&ModelPreference::Test), None, None, 20)
            .expect("the model should save");

        assert_eq!(remodelled.display_name.as_deref(), Some("Moti"));
    }

    #[test]
    fn erasing_the_name_stores_null_not_an_empty_string() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        repository
            .update_user_preferences(None, Some("Moti"), None, 10)
            .expect("the name should save");

        let cleared = repository
            .update_user_preferences(None, Some("   "), None, 20)
            .expect("the name should clear");

        assert_eq!(cleared.display_name, None);
    }

    #[test]
    fn a_name_longer_than_the_cap_is_refused() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        let too_long = "a".repeat(DISPLAY_NAME_MAX_CHARS + 1);

        let error = repository
            .update_user_preferences(None, Some(&too_long), None, 10)
            .expect_err("an oversized name should be refused");

        assert!(error.to_string().contains("60"), "got: {error}");
        assert_eq!(
            repository
                .get_user_preferences()
                .expect("preferences should still read")
                .display_name,
            None,
            "a refused name must not be half-written",
        );
    }

    #[test]
    fn the_cap_counts_characters_not_bytes() {
        // A Hebrew name is two bytes a letter; a byte cap would halve it.
        let hebrew = "מ".repeat(DISPLAY_NAME_MAX_CHARS);
        assert_eq!(
            normalize_display_name(&hebrew).expect("a 60-character name should pass"),
            Some(hebrew),
        );
    }

    fn built_in_companion_id(database: &ScratchDatabase) -> String {
        let connection =
            rusqlite::Connection::open(&database.path).expect("the scratch db should open");
        connection
            .query_row(
                "SELECT id FROM companions WHERE is_built_in = 1",
                [],
                |row| row.get(0),
            )
            .expect("the migrations seed a built-in companion")
    }

    #[test]
    fn a_fresh_install_has_no_picked_companion() {
        let database = ScratchDatabase::new();
        let preferences = database
            .repository()
            .get_user_preferences()
            .expect("defaults should be readable");
        assert_eq!(preferences.default_companion_id, None);
    }

    #[test]
    fn picking_a_companion_persists_and_leaves_the_rest_alone() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        let built_in = built_in_companion_id(&database);
        repository
            .update_user_preferences(None, Some("Moti"), None, 10)
            .expect("the name should save");

        let picked = repository
            .update_user_preferences(None, None, Some(&built_in), 20)
            .expect("the pick should save");

        assert_eq!(picked.default_companion_id.as_deref(), Some(built_in.as_str()));
        assert_eq!(picked.display_name.as_deref(), Some("Moti"));
        assert_eq!(
            repository
                .get_user_preferences()
                .expect("preferences should read back")
                .default_companion_id
                .as_deref(),
            Some(built_in.as_str()),
        );
    }

    #[test]
    fn a_pick_that_names_nobody_is_refused_and_blank_clears() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        let built_in = built_in_companion_id(&database);
        repository
            .update_user_preferences(None, None, Some(&built_in), 10)
            .expect("the pick should save");

        let error = repository
            .update_user_preferences(None, None, Some("no-such-companion"), 20)
            .expect_err("an unknown companion should be refused");
        assert!(error.to_string().contains("no longer exists"), "got: {error}");
        assert_eq!(
            repository
                .get_user_preferences()
                .expect("preferences should still read")
                .default_companion_id
                .as_deref(),
            Some(built_in.as_str()),
            "a refused pick must not touch the stored one",
        );

        let cleared = repository
            .update_user_preferences(None, None, Some("  "), 30)
            .expect("blank should clear");
        assert_eq!(cleared.default_companion_id, None);
    }

    #[test]
    fn a_picked_companion_that_was_deleted_reads_as_unset() {
        let database = ScratchDatabase::new();
        let repository = database.repository();
        let connection =
            rusqlite::Connection::open(&database.path).expect("the scratch db should open");
        connection
            .execute(
                "INSERT INTO companions (id, name, memory_agent_name, created_at, updated_at)
                 VALUES ('companion-gone', 'Gone', 'gone', 1, 1)",
                [],
            )
            .expect("a companion should insert");
        repository
            .update_user_preferences(None, None, Some("companion-gone"), 10)
            .expect("the pick should save");
        connection
            .execute("DELETE FROM companions WHERE id = 'companion-gone'", [])
            .expect("the companion should delete");

        let preferences = repository
            .get_user_preferences()
            .expect("preferences should read");
        assert_eq!(preferences.default_companion_id, None);
    }

    #[test]
    fn scratch_databases_are_named_uniquely() {
        // Guards the harness itself: two ScratchDatabases must not collide.
        let first = ScratchDatabase::new();
        let second = ScratchDatabase::new();
        assert_ne!(first.path.as_path(), second.path.as_path() as &Path);
    }
}
