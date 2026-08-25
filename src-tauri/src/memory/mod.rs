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
use tauri::{ipc::Channel, State};

use crate::{
    app_error::AppError,
    chat::repository::ChatRepository,
    companions::CompanionResolver,
    models::ModelResolver,
    preferences::{ModelPreference, PreferenceRepository, ResolvedVoice},
    secret_vault::SecretVault,
};

const MEMORY_ORGAN_BASE: &str = "http://localhost:8002/api/v1/memory";
const ACCOUNT_TOKEN_REF: &str = "semantix-account-token";

/// The canonical Muninn. Machine-local by construction: nothing outside this
/// box can route to it, so a companion flagged `is_origin` on any other
/// install simply fails to connect rather than reaching someone else's brain.
const MUNINN_BASE: &str = "http://localhost:8005/api/v1";

/// WHICH brain a companion reads and writes — resolved once per turn from the
/// roster, never chosen by the model and never named in a tool argument.
///
/// The two organs disagree about more than a port. The Semantix organ scopes a
/// memory by putting the agent in the PATH and proves the caller with an
/// account bearer; Muninn scopes by CHANNEL in the body and, being local,
/// authenticates nobody. Everything above this enum is spared both facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MemoryTarget {
    /// Account-scoped, on :8002. What every install uses and the only value a
    /// public build can produce.
    Organ { agent_id: String },
    /// The canonical Muninn on :8005, addressed by channel. The channel name
    /// IS the companion's `memory_agent_name`: one companion, one namespace,
    /// structurally unable to read another's.
    ///
    /// The channel is the WHOLE address. Muninn also accepts an `X-Agent-Id`
    /// header for authorship, but it must be a UUID and a companion has no
    /// such id to offer — sending the channel name there is rejected outright
    /// (verified against the live server, not inferred). Carves therefore land
    /// under the server's default author, which is honest: the channel already
    /// says who wrote them.
    Muninn { channel: String },
}

impl MemoryTarget {
    /// Read the roster, not the argument. `agent_ref` is whatever
    /// `ensure_memory_agent` handed the frontend — an organ uuid for a normal
    /// companion, the agent name for an origin one — and only the local
    /// companions table decides which it was.
    fn resolve(companions: &CompanionResolver, agent_ref: &str) -> Result<Self, String> {
        let agent_ref = agent_ref.trim();
        let origin = companions
            .is_origin_agent(agent_ref)
            .map_err(|error| format!("The companion roster could not be read: {error}"))?;
        Ok(if origin {
            Self::Muninn { channel: agent_ref.to_owned() }
        } else {
            Self::Organ { agent_id: agent_ref.to_owned() }
        })
    }
}

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
                companions: CompanionResolver::open(database_path)?,
            }),
        })
    }
}

struct MemoryService {
    chats: ChatRepository,
    preferences: PreferenceRepository,
    models: ModelResolver,
    companions: CompanionResolver,
}

#[derive(Serialize)]
struct SleepTurn {
    role: &'static str,
    text: String,
    /// Already-slept tail sent for the distiller's footing — never carved from.
    context: bool,
}

#[derive(Serialize)]
struct SleepCustomModel {
    base_url: String,
    api_key: String,
    model_id: String,
}

/// One line of the agent's index, sent so a re-extracted fact reuses its exact
/// name (a clean update) instead of minting a near-duplicate slug.
#[derive(Serialize, Deserialize)]
struct SleepExistingMemory {
    name: String,
    description: String,
}

#[derive(Serialize)]
struct SleepRequest {
    turns: Vec<SleepTurn>,
    custom_model: SleepCustomModel,
    project_tag: Option<String>,
    existing: Vec<SleepExistingMemory>,
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
    /// True when the ledger left nothing to distill — no organ call was made.
    #[serde(default)]
    nothing_new: bool,
    /// Whose hand wrote these memories, when it wasn't the companion's own
    /// model. The organ never sends this — Companion fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scribe_note: Option<String>,
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
    /// The un-slept message ids this pass consumes — stamped into the ledger
    /// (`messages.slept_at`) only after the organ confirms the pass landed.
    fresh_message_ids: Vec<String>,
    /// Set when a model other than the companion's own wrote the memories —
    /// surfaced in the outcome so the substitution is never silent.
    scribe_note: Option<String>,
}

/// How many already-slept messages ride along as context-only tail.
const CONTEXT_TAIL_MESSAGES: usize = 4;

impl MemoryService {
    /// `Ok(None)` = the conversation has turns, but the ledger already claims
    /// them all — nothing new to distill, no organ call needed.
    fn prepare_sleep(
        &self,
        conversation_id: &str,
        agent_id: &str,
    ) -> Result<Option<PreparedSleep>, AppError> {
        let bearer = load_account_token()?
            .ok_or_else(|| AppError::validation("Connect your Semantix account before sleeping."))?;

        let thread = self
            .chats
            .get_thread(conversation_id)?
            .ok_or_else(|| AppError::validation("That conversation no longer exists."))?;

        let conversational: Vec<_> = thread
            .messages
            .iter()
            .filter(|m| {
                m.status == "completed"
                    && !m.content.trim().is_empty()
                    && matches!(m.role.as_str(), "user" | "assistant")
            })
            .collect();
        if conversational.is_empty() {
            return Err(AppError::validation("There is nothing to sleep yet — say something first."));
        }

        let (slept, fresh): (Vec<_>, Vec<_>) = conversational
            .into_iter()
            .partition(|m| m.slept_at.is_some());
        if fresh.is_empty() {
            return Ok(None);
        }

        // A short already-slept tail keeps the distiller's footing on a
        // follow-up pass; the organ renders it under a do-NOT-carve header.
        let tail_start = slept.len().saturating_sub(CONTEXT_TAIL_MESSAGES);
        let as_turn = |m: &crate::chat::Message, context: bool| SleepTurn {
            role: if m.role == "user" { "user" } else { "assistant" },
            text: m.content.clone(),
            context,
        };
        let turns: Vec<SleepTurn> = slept[tail_start..]
            .iter()
            .map(|&m| as_turn(m, true))
            .chain(fresh.iter().map(|&m| as_turn(m, false)))
            .collect();
        let fresh_message_ids = fresh.iter().map(|m| m.id.clone()).collect();

        // The distiller speaks with the companion's voice, the same one that
        // answered in the thread — so /sleep and chat can never diverge.
        let companion = self
            .companions
            .resolve(thread.conversation.companion_id.as_deref())?;
        // WHICH BRAIN never changes — the memory lands in this companion's own
        // agent either way. Only the SCRIBE can differ: the organ distills on
        // an OpenAI-compatible base_url + key, and a Claude Code companion
        // authenticates with a local login that cannot travel to the server.
        // So a Claude companion borrows the user's default model to write, and
        // the outcome says whose hand held the pen — a silent substitution
        // would be the dishonest part, not the substitution itself.
        let (configured_model_id, scribe_note) = match self
            .preferences
            .resolve_voice(&companion.model_preference)?
        {
            ResolvedVoice::Configured(model_id) => (model_id, None),
            ResolvedVoice::ClaudeCode(_) => {
                // The user's default first; failing that, their most recently
                // touched configured model. Refusing while a usable model sits
                // right there would be pedantry, not caution — the note names
                // whichever one wrote, so a wrong guess is visible and the fix
                // is to set a default.
                let borrowed = match self.preferences.resolve_voice(&ModelPreference::Inherit)? {
                    ResolvedVoice::Configured(model_id) => model_id,
                    _ => self
                        .models
                        .list()?
                        .into_iter()
                        .next()
                        .map(|model| model.id)
                        .ok_or_else(|| {
                            AppError::validation(
                                "Claude Code can't run the distiller, and there is no Semantix model to borrow — add one in Settings, then sleep.",
                            )
                        })?,
                };
                let name = self
                    .models
                    .list()?
                    .into_iter()
                    .find(|model| model.id == borrowed)
                    .map(|model| model.display_name)
                    .unwrap_or_else(|| borrowed.clone());
                (
                    borrowed,
                    Some(format!(
                        "Claude Code can't run the distiller, so {name} wrote these memories into this companion's own brain."
                    )),
                )
            }
            ResolvedVoice::TestStream => {
                return Err(AppError::validation(
                    "Give this companion a real model before sleeping — the test stream cannot distill memories.",
                ))
            }
        };
        let model = self.models.resolve(&configured_model_id)?;
        let base_url = provider_base_url(&model.provider_id)?;

        Ok(Some(PreparedSleep {
            body: SleepRequest {
                turns,
                custom_model: SleepCustomModel {
                    base_url: base_url.to_owned(),
                    api_key: model.api_key.to_string(),
                    model_id: model.model_id,
                },
                project_tag: Some("companion".to_owned()),
                existing: Vec::new(),
            },
            bearer,
            agent_id: agent_id.to_owned(),
            fresh_message_ids,
            scribe_note,
        }))
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

/// The carve payload, spoken as Muninn speaks it.
///
/// A rename, not a remap — the two organs want the same five facts under three
/// different names. `mem_type` is `type` here and is REQUIRED where the organ
/// let it default, so a carve that names no type is filed as `episodic`: the
/// honest label for something a conversation produced. `project_tag` has no
/// counterpart because `channel` already does that work, and it carries the
/// isolation the tag only hinted at.
fn muninn_carve_body(payload: &serde_json::Value, channel: &str) -> serde_json::Value {
    let text = |key: &str| payload.get(key).and_then(|value| value.as_str()).unwrap_or_default();
    let mut body = serde_json::json!({
        "name": text("name"),
        "description": text("description"),
        "body": text("body"),
        "type": payload
            .get("mem_type")
            .and_then(|value| value.as_str())
            .unwrap_or("episodic"),
        "channel": channel,
    });
    if let Some(importance) = payload.get("importance").and_then(|value| value.as_f64()) {
        body["importance"] = serde_json::json!(importance);
    }
    body
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
    state: State<'_, MemoryState>,
    name: String,
    description: String,
) -> Result<MemoryAgentDto, String> {
    // An origin companion has no roster round-trip to make. Muninn materialises
    // a channel on first write, so there is nothing to create and nothing that
    // can fail here — and crucially no bearer to demand, which is what would
    // otherwise stop an origin companion from ever reaching its memory without
    // a Semantix account it does not need.
    //
    // The synthetic id is the agent NAME, which is what every later call reads
    // back as `agent_ref` and what `MemoryTarget::resolve` recognises.
    if MemoryTarget::resolve(&state.service.companions, &name)?
        != (MemoryTarget::Organ { agent_id: name.trim().to_owned() })
    {
        let now = "";
        return Ok(MemoryAgentDto {
            agent_id: name.trim().to_owned(),
            name: name.trim().to_owned(),
            description,
            memory_count: 0,
            created_at: now.to_owned(),
            updated_at: now.to_owned(),
        });
    }

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
    state: State<'_, MemoryState>,
    agent_id: String,
    query: String,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    let target = MemoryTarget::resolve(&state.service.companions, &agent_id)?;
    let client = reqwest::Client::new();
    let limit = limit.unwrap_or(8);

    let request = match &target {
        MemoryTarget::Organ { agent_id } => client
            .post(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/recall"))
            .bearer_auth(organ_bearer().await?)
            .json(&serde_json::json!({
                "query": query,
                "limit": limit,
                "project_tag": "companion",
            })),
        MemoryTarget::Muninn { channel } => client
            .post(format!("{MUNINN_BASE}/recall"))
            .json(&serde_json::json!({
                "query": query,
                "limit": limit,
                "channel": channel,
            })),
    };

    let response = request
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

/// Write one memory into the organ — the pen behind the model's `carve_memory`
/// tool. Not a command: the chat tool loop is the only caller. The organ
/// upserts by name (created=false means an existing memory was overwritten)
/// and embeds server-side; project_tag pins the carve to companion's shelf.
pub(crate) async fn write_memory(
    target: &MemoryTarget,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let request = match target {
        MemoryTarget::Organ { agent_id } => client
            .post(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/memories"))
            .bearer_auth(organ_bearer().await?)
            .json(payload),
        MemoryTarget::Muninn { channel } => client
            .post(format!("{MUNINN_BASE}/memories"))
            .json(&muninn_carve_body(payload, channel)),
    };
    let response = request
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let detail = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .map(|body| body.get("detail").cloned().unwrap_or(body).to_string())
            .unwrap_or_default();
        return Err(format!("memory write failed: HTTP {} {detail}", status.as_u16()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("The write result could not be read: {error}"))
}

/// One full memory by its exact name — the drill-down behind the model's
/// `recall_memory` tool. Not a command: the chat tool loop is the only caller.
pub(crate) async fn fetch_memory(
    target: &MemoryTarget,
    name: &str,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let request = match target {
        MemoryTarget::Organ { agent_id } => client
            .get(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/memories/{name}"))
            .bearer_auth(organ_bearer().await?),
        MemoryTarget::Muninn { channel } => client
            .get(format!("{MUNINN_BASE}/memories/{name}"))
            .query(&[("channel", channel.as_str())]),
    };
    let response = request
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Err(format!("no memory named \"{name}\" exists"));
    }
    if !status.is_success() {
        return Err(format!("memory fetch failed: HTTP {}", status.as_u16()));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("The memory could not be read: {error}"))
}

/// The agent's index (names + descriptions, recent-first, non-archived,
/// capped) for the distiller's name-reuse leash. Fail-open: a dead fetch
/// mutes the leash, never blocks the sleep.
async fn fetch_existing_index(bearer: &str, agent_id: &str) -> Vec<SleepExistingMemory> {
    const CAP: usize = 200;
    #[derive(Deserialize)]
    struct IndexItem {
        name: String,
        description: String,
        archived_at: Option<String>,
    }
    let response = reqwest::Client::new()
        .get(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/memories"))
        .bearer_auth(bearer)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await;
    let Ok(response) = response else { return Vec::new() };
    if !response.status().is_success() {
        return Vec::new();
    }
    match response.json::<Vec<IndexItem>>().await {
        Ok(items) => items
            .into_iter()
            .filter(|item| item.archived_at.is_none())
            .take(CAP)
            .map(|item| SleepExistingMemory { name: item.name, description: item.description })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Consume the organ's /sleep/stream SSE body, relaying each stage frame to
/// the progress channel; returns the terminal outcome. `Ok(None)` = the
/// stream route doesn't exist (older organ) — caller falls back to the
/// plain POST.
async fn sleep_via_stream(
    prepared: &PreparedSleep,
    on_progress: &Channel<serde_json::Value>,
) -> Result<Option<SleepOutcome>, String> {
    use futures_util::StreamExt;

    let response = reqwest::Client::new()
        .post(format!(
            "{MEMORY_ORGAN_BASE}/agents/{}/sleep/stream",
            prepared.agent_id
        ))
        .bearer_auth(&prepared.bearer)
        .json(&prepared.body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|error| format!("The memory organ could not be reached: {error}"))?;

    let status = response.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(format!("Sleep failed: HTTP {status}"));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut outcome: Option<SleepOutcome> = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("The sleep stream broke: {error}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        // SSE frames split on a blank line; comment/keepalive frames carry no data.
        while let Some(boundary) = buffer.find("\n\n") {
            let frame = buffer[..boundary].to_owned();
            buffer.drain(..boundary + 2);
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&data) else {
                continue;
            };
            match event.get("type").and_then(|t| t.as_str()) {
                Some("complete") => {
                    outcome = serde_json::from_value::<SleepOutcome>(event.clone()).ok();
                }
                Some("error") => {
                    let detail = event
                        .get("detail")
                        .and_then(|d| d.as_str())
                        .unwrap_or("the sleep pass failed");
                    return Err(format!("Sleep failed: {detail}"));
                }
                _ => {}
            }
            let _ = on_progress.send(event);
        }
    }
    outcome
        .map(Some)
        .ok_or_else(|| "The sleep stream ended without a result.".to_owned())
}

/// The plain POST — the pre-stream door, kept as the fallback for an organ
/// without /sleep/stream.
async fn sleep_via_post(prepared: &PreparedSleep) -> Result<SleepOutcome, String> {
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

#[tauri::command]
pub(crate) async fn sleep_conversation(
    state: State<'_, MemoryState>,
    conversation_id: String,
    agent_id: String,
    on_progress: Channel<serde_json::Value>,
) -> Result<SleepOutcome, String> {
    // Sleep does not cross. The organ distils through `/agents/{id}/sleep`;
    // Muninn has no agent-scoped sleep route at all — its own pass is a
    // different design, not a renamed endpoint — so there is nothing to point
    // this at. Refuse in one clear sentence rather than fail as a 404 the user
    // would read as a broken connection.
    if let MemoryTarget::Muninn { .. } =
        MemoryTarget::resolve(&state.service.companions, &agent_id)?
    {
        return Err(
            "This companion remembers through the canonical Muninn, which distils as it goes \
             rather than at the end of a conversation. There is no sleep pass to run."
                .to_owned(),
        );
    }

    let service = Arc::clone(&state.service);
    let prepare_conversation_id = conversation_id.clone();
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        service.prepare_sleep(&prepare_conversation_id, &agent_id)
    })
    .await
    .map_err(|error| format!("Sleep task failed: {error}"))?
    .map_err(String::from)?;

    // The ledger already claims every turn — honest no-op, no organ call.
    let Some(mut prepared) = prepared else {
        return Ok(SleepOutcome {
            created: 0,
            updated: 0,
            dropped: 0,
            memories: Vec::new(),
            nothing_new: true,
            scribe_note: None,
        });
    };

    prepared.body.existing = fetch_existing_index(&prepared.bearer, &prepared.agent_id).await;

    let mut outcome = match sleep_via_stream(&prepared, &on_progress).await? {
        Some(outcome) => outcome,
        None => sleep_via_post(&prepared).await?,
    };
    // Whose hand wrote them, when it wasn't the companion's own model.
    outcome.scribe_note = prepared.scribe_note.take();

    // Stamp the ledger only now that the organ confirmed the pass landed.
    let service = Arc::clone(&state.service);
    let stamped_ids = prepared.fresh_message_ids;
    let stamp_result = tauri::async_runtime::spawn_blocking(move || {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default();
        service.chats.mark_messages_slept(&stamped_ids, timestamp)
    })
    .await
    .map_err(|error| format!("Sleep ledger task failed: {error}"))?;
    if let Err(error) = stamp_result {
        // The memories landed; a failed stamp only means a benign re-distill
        // next pass (writes upsert by name). Surface it without failing.
        eprintln!("[memory] sleep ledger stamp failed: {error}");
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::{muninn_carve_body, MemoryTarget, MUNINN_BASE};

    /// The organ let `mem_type` default; Muninn requires `type`. A carve that
    /// names none must still be filable, or the model loses the tool on its
    /// first sloppy call.
    #[test]
    fn a_carve_without_a_type_is_filed_as_episodic() {
        let payload = serde_json::json!({
            "name": "a-thing",
            "description": "one line",
            "body": "the fact",
            "project_tag": "companion",
        });
        let body = muninn_carve_body(&payload, "arc");

        assert_eq!(body["type"], "episodic");
        assert_eq!(body["channel"], "arc");
        assert_eq!(body["name"], "a-thing");
        assert!(body.get("project_tag").is_none(), "the tag has no counterpart");
        assert!(body.get("importance").is_none(), "an unset importance stays unset");
    }

    #[test]
    fn a_carve_carries_its_type_and_importance_across() {
        let payload = serde_json::json!({
            "name": "a-thing",
            "description": "one line",
            "body": "the fact",
            "mem_type": "insight",
            "importance": 0.75,
        });
        let body = muninn_carve_body(&payload, "arc");

        assert_eq!(body["type"], "insight");
        assert_eq!(body["importance"], 0.75);
    }

    /// The whole safety story in one line: the local brain is addressed by a
    /// loopback URL, so an origin flag on anyone else's machine reaches
    /// nothing rather than reaching someone.
    #[test]
    fn the_local_brain_is_only_ever_addressed_over_loopback() {
        assert!(MUNINN_BASE.starts_with("http://localhost:"));
    }

    #[test]
    fn the_two_backends_are_never_equal() {
        let organ = MemoryTarget::Organ { agent_id: "arc".to_owned() };
        let muninn = MemoryTarget::Muninn { channel: "arc".to_owned() };
        assert_ne!(organ, muninn, "same string, different brain");
    }
}
