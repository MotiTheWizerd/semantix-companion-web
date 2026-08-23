mod app_error;
mod chat;
mod companions;
mod credentials;
mod database;
mod inference;
mod memory;
mod tools;
mod web;
mod models;
mod preferences;
mod secret_vault;
mod streaming;

use std::fs;

use chat::ChatState;
use companions::CompanionState;
use credentials::CredentialState;
use memory::MemoryState;
use models::ModelState;
use preferences::PreferenceState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app.path().app_local_data_dir()?;
            fs::create_dir_all(&app_data_dir)?;
            let database_path = app_data_dir.join("companion.db");
            database::initialise(&database_path)?;
            let credential_state = CredentialState::open(&database_path)?;
            let model_state = ModelState::open(&database_path)?;
            let preference_state = PreferenceState::open(&database_path)?;
            let chat_state = ChatState::open(&database_path)?;
            let memory_state = MemoryState::open(&database_path)?;
            let companion_state = CompanionState::open(&database_path)?;
            app.manage(companion_state);
            app.manage(credential_state);
            app.manage(model_state);
            app.manage(preference_state);
            app.manage(chat_state);
            app.manage(memory_state);
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
            memory::sleep_conversation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Semantix Companion");
}
