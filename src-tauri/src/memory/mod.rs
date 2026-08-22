// The memory seam — Companion's door to the Semantix memory organ on :8002.
//
// EVERY organ call runs here, never as a webview fetch: the organ's CORS
// allowlist knows the studio's origin, not Companion's, so a browser-side
// request dies with WebKit's opaque "Load failed" (proven s484). The recall
// commands are thin proxies — bearer from the vault, JSON through untouched —
// while /sleep stays a full backend pass because it distills on the USER'S
// model key, which never leaves the vault.

use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use tauri::State;

use crate::{
    app_error::AppError,
    chat::repository::ChatRepository,
    models::ModelResolver,
    preferences::PreferenceRepository,
    secret_vault::SecretVault,
};

const MEMORY_ORGAN_BASE: &str = "http://localhost:8002/api/v1/memory";
const ACCOUNT_TOKEN_REF: &str = "semantix-account-token";

/// Chat/completions bases per provider — the organ's sleep model speaks
/// OpenAI-compat, so only providers with such an endpoint can distill.
fn provider_base_url(provider_id: &str) -> Result<&'static str, AppError> {
    match provider_id {
        "together" => Ok("https://api.together.ai/v1"),
        "test" => Err(AppError::validation(
            "Pick a real model for this conversation before sleeping — the test stream cannot distill memories.",
        )),
        other => Err(AppError::validation(format!(
            "Provider \"{other}\" has no known endpoint for the memory sleep pass.",
        ))),
    }
}

pub(crate) struct MemoryState {
    service: Arc<MemoryService>,
}

impl MemoryState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(MemoryService {
                chats: ChatRepository::open(database_path)?,
                preferences: PreferenceRepository::open(database_path)?,
                models: ModelResolver::open(database_path)?,
            }),
        })
    }
}

struct MemoryService {
    chats: ChatRepository,
    preferences: PreferenceRepository,
    models: ModelResolver,
}

#[derive(Serialize)]
struct SleepTurn {
    role: &'static str,
    text: String,
}

#[derive(Serialize)]
struct SleepCustomModel {
    base_url: String,
    api_key: String,
    model_id: String,
}

#[derive(Serialize)]
struct SleepRequest {
    turns: Vec<SleepTurn>,
    custom_model: SleepCustomModel,
    project_tag: Option<String>,
}

#[derive(Deserialize)]
struct SleepMemoryName {
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SleepOutcome {
    created: u32,
    updated: u32,
    dropped: u32,
    #[serde(deserialize_with = "memory_names", rename = "memories")]
    memories: Vec<String>,
}

fn memory_names<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<SleepMemoryName>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|m| m.name).collect())
}

/// Everything the sleep POST needs, gathered while the vault is open —
/// assembled inside spawn_blocking (rusqlite + keyring are sync), sent async.
struct PreparedSleep {
    body: SleepRequest,
    bearer: String,
    agent_id: String,
}

impl MemoryService {
    fn prepare_sleep(&self, conversation_id: &str, agent_id: &str) -> Result<PreparedSleep, AppError> {
        let bearer = load_account_token()?
            .ok_or_else(|| AppError::validation("Connect your Semantix account before sleeping."))?;

        let thread = self
            .chats
            .get_thread(conversation_id)?
            .ok_or_else(|| AppError::validation("That conversation no longer exists."))?;

        let turns: Vec<SleepTurn> = thread
            .messages
            .iter()
            .filter(|m| m.status == "completed" && !m.content.trim().is_empty())
            .filter_map(|m| match m.role.as_str() {
                "user" => Some(SleepTurn { role: "user", text: m.content.clone() }),
                "assistant" => Some(SleepTurn { role: "assistant", text: m.content.clone() }),
                _ => None,
            })
            .collect();
        if turns.is_empty() {
            return Err(AppError::validation("There is nothing to sleep yet — say something first."));
        }

        let configured_model_id = self
            .preferences
            .resolve_model_id(&thread.conversation.model_preference)?
            .ok_or_else(|| {
                AppError::validation(
                    "Pick a real model for this conversation before sleeping — the test stream cannot distill memories.",
                )
            })?;
        let model = self.models.resolve(&configured_model_id)?;
        let base_url = provider_base_url(&model.provider_id)?;

        Ok(PreparedSleep {
            body: SleepRequest {
                turns,
                custom_model: SleepCustomModel {
                    base_url: base_url.to_owned(),
                    api_key: model.api_key.to_string(),
                    model_id: model.model_id,
                },
                project_tag: Some("companion".to_owned()),
            },
            bearer,
            agent_id: agent_id.to_owned(),
        })
    }
}

fn load_account_token() -> Result<Option<String>, AppError> {
    Ok(SecretVault::try_get(ACCOUNT_TOKEN_REF)?.map(|secret| secret.to_string()))
}

#[tauri::command]
pub(crate) async fn set_memory_account_token(token: String) -> Result<(), String> {
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err(AppError::validation("Paste a token before saving.").into());
    }
    tauri::async_runtime::spawn_blocking(move || SecretVault::store(ACCOUNT_TOKEN_REF, &token))
        .await
        .map_err(|error| format!("Vault task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn get_memory_account_token() -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(load_account_token)
        .await
        .map_err(|error| format!("Vault task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn clear_memory_account_token() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| SecretVault::delete(ACCOUNT_TOKEN_REF))
        .await
        .map_err(|error| format!("Vault task failed: {error}"))?
        .map_err(String::from)
}

/// One agent row as the organ speaks it; `memory_count` defaults because the
/// create response omits it (a fresh agent holds nothing).
#[derive(Deserialize, Serialize)]
pub(crate) struct MemoryAgentDto {
    agent_id: String,
    name: String,
    description: String,
    #[serde(default)]
    memory_count: u32,
    created_at: String,
    updated_at: String,
}

async fn organ_bearer() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(load_account_token)
        .await
        .map_err(|error| format!("Vault task failed: {error}"))?
        .map_err(String::from)?
        .ok_or_else(|| "Connect your Semantix account first.".to_owned())
}

#[tauri::command]
pub(crate) async fn ensure_memory_agent(
    name: String,
    description: String,
) -> Result<MemoryAgentDto, String> {
    let bearer = organ_bearer().await?;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{MEMORY_ORGAN_BASE}/agents"))
        .bearer_auth(&bearer)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("memory roster failed: HTTP {}", response.status().as_u16()));
    }
    let agents: Vec<MemoryAgentDto> = response
        .json()
        .await
        .map_err(|error| format!("The memory roster could not be read: {error}"))?;
    if let Some(existing) = agents.into_iter().find(|agent| agent.name == name) {
        return Ok(existing);
    }

    let response = client
        .post(format!("{MEMORY_ORGAN_BASE}/agents"))
        .bearer_auth(&bearer)
        .json(&serde_json::json!({ "name": name, "description": description }))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;
    let status = response.status();
    if status.as_u16() == 409 {
        return Err(format!("agent \"{name}\" already exists"));
    }
    if !status.is_success() {
        return Err(format!("create agent failed: HTTP {}", status.as_u16()));
    }
    response
        .json::<MemoryAgentDto>()
        .await
        .map_err(|error| format!("The created agent could not be read: {error}"))
}

/// Pure pipe: the hit list is the frontend's to interpret, so the JSON passes
/// through untyped — the organ's schema can grow without touching Rust.
#[tauri::command]
pub(crate) async fn recall_memories(
    agent_id: String,
    query: String,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let bearer = organ_bearer().await?;
    let response = reqwest::Client::new()
        .post(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/recall"))
        .bearer_auth(&bearer)
        .json(&serde_json::json!({
            "query": query,
            "limit": limit.unwrap_or(8),
            "project_tag": "companion",
        }))
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("recall failed: HTTP {}", status.as_u16()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("The recall result could not be read: {error}"))
}

#[tauri::command]
pub(crate) async fn sleep_conversation(
    state: State<'_, MemoryState>,
    conversation_id: String,
    agent_id: String,
) -> Result<SleepOutcome, String> {
    let service = Arc::clone(&state.service);
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        service.prepare_sleep(&conversation_id, &agent_id)
    })
    .await
    .map_err(|error| format!("Sleep task failed: {error}"))?
    .map_err(String::from)?;

    let response = reqwest::Client::new()
        .post(format!("{MEMORY_ORGAN_BASE}/agents/{}/sleep", prepared.agent_id))
        .bearer_auth(&prepared.bearer)
        .json(&prepared.body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        let detail = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body.get("detail")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(format!("Sleep failed: {detail}"));
    }
    response
        .json::<SleepOutcome>()
        .await
        .map_err(|error| format!("The sleep result could not be read: {error}"))
}
