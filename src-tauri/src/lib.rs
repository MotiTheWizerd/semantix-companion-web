mod app_error;
mod chat;
mod credentials;
mod database;
mod models;
mod secret_vault;
mod streaming;

use std::fs;

use chat::ChatState;
use credentials::CredentialState;
use models::ModelState;
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
            let chat_state = ChatState::open(&database_path)?;
            app.manage(credential_state);
            app.manage(model_state);
            app.manage(chat_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            credentials::list_known_model_providers,
            credentials::list_provider_credentials,
            credentials::create_provider_credential,
            credentials::delete_provider_credential,
            models::list_configured_models,
            models::create_configured_model,
            models::delete_configured_model,
            chat::list_conversations,
            chat::get_conversation_thread,
            chat::submit_message,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Semantix Companion");
}
