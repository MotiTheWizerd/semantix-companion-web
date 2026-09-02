mod agent_mail;
mod app_error;
mod chat;
mod companions;
mod credentials;
mod database;
mod import;
mod inference;
mod memory;
mod tools;
mod web;
mod models;
mod preferences;
mod raven_calls;
mod secret_vault;
mod streaming;
mod styles;

use std::fs;
use std::path::{Path, PathBuf};

use chat::ChatState;
use companions::CompanionState;
use credentials::CredentialState;
use memory::MemoryState;
use models::ModelState;
use preferences::PreferenceState;
use raven_calls::RavenCallState;
use styles::StyleState;
use tauri::Manager;

/// `~/.semantix/companion/companion.db` — the Companion's ONE database, at a
/// FIXED path anchored to the real home directory.
///
/// ⚑ IT USED TO LIVE IN TAURI'S `app_local_data_dir()`, AND THAT WAS A DATA-LOSS
/// BUG, not a detail. That directory resolves through `$XDG_DATA_HOME`, and
/// VSCode ships as a snap which redirects that variable to
/// `~/snap/code/<REVISION>/.local/share` — with the snap REVISION NUMBER in the
/// path. So every VSCode update silently handed the Companion a brand-new empty
/// database, and a Companion launched outside the IDE saw a third one. Measured
/// on this machine before the fix: snap 257 held schema 4 with 23 messages,
/// snap 258 held schema 17 with 523, and mimir could only ever read whichever
/// file happened to be freshest. `$HOME` itself is NOT redirected, which is why
/// anchoring to it is the fix.
///
/// `~/.semantix/` is where the rest of Semantix already keeps its global state
/// (`conversations.db`, `analytics.db`, `USER/`, `trig/`) for exactly this
/// reason — see semantix-server's `global_data::paths`, and `semantix-trig`,
/// which was moved off `dirs::data_dir()` after the same snap bug bit it. The
/// Companion now joins them.
fn resolve_database_path(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let directory = app.path().home_dir()?.join(".semantix").join("companion");
    fs::create_dir_all(&directory)?;
    let path = directory.join("companion.db");
    // One-shot, first-launch-only: carry whatever the old XDG-derived location
    // holds. Once the fixed path exists it is the only one that is ever read,
    // so a later VSCode update cannot resurrect an empty database.
    if !path.exists() {
        if let Ok(legacy_dir) = app.path().app_local_data_dir() {
            adopt_legacy_database(&legacy_dir.join("companion.db"), &path)?;
        }
    }
    Ok(path)
}

/// Move a pre-`~/.semantix` database into its new home, sidecars and all.
///
/// The `-wal` is the half that must not be left behind: the app may have been
/// closed without a checkpoint, and every write since the last one lives only
/// there. `-shm` is regenerable but travels too, so no stale shared index is
/// left pointing at a file that has moved.
fn adopt_legacy_database(from: &Path, to: &Path) -> std::io::Result<()> {
    if !from.exists() {
        return Ok(());
    }
    move_file(from, to)?;
    for suffix in ["-wal", "-shm"] {
        let sidecar = with_suffix(from, suffix);
        if sidecar.exists() {
            move_file(&sidecar, &with_suffix(to, suffix))?;
        }
    }
    Ok(())
}

/// `rename` cannot cross a filesystem boundary, and snap's redirected data dir
/// is not guaranteed to sit on the same mount as `~/.semantix`. Fall back to a
/// copy so the migration does not depend on the layout of the machine.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to)?;
    fs::remove_file(from)
}

/// `foo.db` + `-wal` → `foo.db-wal`. Appends to the path itself rather than
/// touching the extension, which is what SQLite actually names its sidecars.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut raw = path.as_os_str().to_owned();
    raw.push(suffix);
    PathBuf::from(raw)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The Claude lane's sidecar ships as a bundle resource. Resolve it
            // here — this is the only place with an AppHandle — and hand it to
            // the provider, which has no way to ask for one itself. A dev run
            // has no resource dir; the lane falls back to the repo copy.
            if let Ok(resource_dir) = app.path().resource_dir() {
                inference::set_bundled_sidecar_dir(resource_dir.join("sidecar"));
            }
            let database_path = resolve_database_path(app)?;
            database::initialise(&database_path)?;
            let credential_state = CredentialState::open(&database_path)?;
            let model_state = ModelState::open(&database_path)?;
            let preference_state = PreferenceState::open(&database_path)?;
            let chat_state = ChatState::open(&database_path)?;
            let memory_state = MemoryState::open(&database_path)?;
            // The import worker distills through the memory seam, so it opens
            // on the same service — and parks any job a dead process left
            // `running`, so the UI offers resume instead of a phantom run.
            let import_state = import::worker::ImportState::open(&database_path, memory_state.service())?;
            let companion_state = CompanionState::open(&database_path)?;
            let style_state = StyleState::open(&database_path)?;
            let raven_call_state = RavenCallState::open(&database_path)?;
            app.manage(import_state);
            app.manage(companion_state);
            app.manage(style_state);
            app.manage(credential_state);
            app.manage(model_state);
            app.manage(preference_state);
            app.manage(memory_state);
            // ⚑ THE WAKER IS STARTED HERE AND NOWHERE ELSE. Everything above
            // this line is a system that still needs a person: calls could be
            // placed, stored and rendered, and they sat in the table until
            // someone pressed enter. This is the loop that reads the table and
            // gives a companion a turn on its own.
            let waker_calls = raven_call_state.repository();
            let waker_chat = chat_state.service();
            app.manage(chat_state);
            app.manage(raven_call_state);
            raven_calls::spawn_waker(app.handle().clone(), waker_calls, waker_chat);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            credentials::list_known_model_providers,
            credentials::list_provider_credentials,
            credentials::create_provider_credential,
            credentials::update_provider_credential,
            credentials::delete_provider_credential,
            models::list_configured_models,
            models::create_configured_model,
            models::update_configured_model,
            models::delete_configured_model,
            companions::list_companions,
            companions::create_companion,
            companions::update_companion,
            companions::delete_companion,
            styles::list_styles,
            styles::get_style_exemplars,
            styles::create_style,
            styles::update_style,
            styles::delete_style,
            styles::harvest::inspect_style_source,
            styles::harvest::harvest_style_exemplars,
            preferences::get_user_preferences,
            preferences::update_user_preferences,
            chat::list_conversations,
            chat::get_conversation_thread,
            chat::update_conversation_companion,
            chat::submit_message,
            memory::set_memory_account_token,
            memory::get_memory_account_token,
            memory::clear_memory_account_token,
            memory::ensure_memory_agent,
            memory::recall_memories,
            memory::load_memory_graph,
            memory::read_memory,
            memory::sleep_conversation,
            import::inspect_import_source,
            import::worker::start_import,
            import::worker::pause_import,
            import::worker::resume_import,
            import::worker::cancel_import,
            import::worker::retry_failed_import,
            import::worker::list_import_jobs,
            raven_calls::list_conversation_calls,
            raven_calls::retry_call_wake,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Semantix Companion");
}
