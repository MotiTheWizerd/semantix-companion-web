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
    inference::api_provider_spec,
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
    /// The channel is the whole ADDRESS; `agent_id` is the SIGNATURE. Muninn
    /// takes authorship from an `X-Agent-Id` header, which must be a UUID on
    /// its holy list (verified against the live server, not inferred — the
    /// channel name there is rejected outright).
    ///
    /// `None` was the s508 shape and it was wrong: a carve with no author is
    /// stored NULL, and the per-prompt recall renders a NULL author as "an
    /// unidentified raven, NOT you". Two carvings made through the Companion
    /// on s509 came back to studio-raven minutes later flagged as a stranger's
    /// lived experience. The channel says WHERE a memory lives; only the
    /// header says who lived it.
    Muninn { channel: String, agent_id: Option<String> },
}

impl MemoryTarget {
    /// Read the roster, not the argument. `agent_ref` is whatever
    /// `ensure_memory_agent` handed the frontend — an organ uuid for a normal
    /// companion, the agent name for an origin one — and only the local
    /// companions table decides which it was.
    fn resolve(companions: &CompanionResolver, agent_ref: &str) -> Result<Self, String> {
        let agent_ref = agent_ref.trim();
        let origin = companions
            .origin_identity(agent_ref)
            .map_err(|error| format!("The companion roster could not be read: {error}"))?;
        Ok(match origin {
            Some(identity) => Self::Muninn {
                channel: agent_ref.to_owned(),
                agent_id: identity.agent_id,
            },
            None => Self::Organ { agent_id: agent_ref.to_owned() },
        })
    }
}

/// Chat/completions bases per provider — the organ's sleep model speaks
/// OpenAI-compat, so only providers with such an endpoint can distill.
fn provider_base_url(provider_id: &str) -> Result<&'static str, AppError> {
    if provider_id == "test" {
        return Err(AppError::validation(
            "Pick a real model for this conversation before sleeping — the test stream cannot distill memories.",
        ));
    }
    api_provider_spec(provider_id)
        .map(|provider| provider.api_base_url)
        .ok_or_else(|| {
            AppError::validation(format!(
                "Provider \"{provider_id}\" has no known endpoint for the memory sleep pass."
            ))
        })
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

    /// The service behind the commands — the import worker distills through
    /// the same seam, never around it.
    pub(crate) fn service(&self) -> Arc<MemoryService> {
        Arc::clone(&self.service)
    }
}

pub(crate) struct MemoryService {
    chats: ChatRepository,
    preferences: PreferenceRepository,
    models: ModelResolver,
    companions: CompanionResolver,
}

#[derive(Serialize)]
pub(crate) struct SleepTurn {
    pub(crate) role: &'static str,
    pub(crate) text: String,
    /// Already-slept tail sent for the distiller's footing — never carved from.
    pub(crate) context: bool,
}

#[derive(Serialize)]
pub(crate) struct SleepCustomModel {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model_id: String,
}

/// One line of the agent's index, sent so a re-extracted fact reuses its exact
/// name (a clean update) instead of minting a near-duplicate slug.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SleepExistingMemory {
    name: String,
    description: String,
}

/// Where an imported conversation came from — sent alongside the turns so the
/// organ's import-mode prompt can era-stamp what it carves. An organ without
/// that prompt simply ignores the field (pydantic drops unknown keys).
#[derive(Serialize)]
pub(crate) struct SleepImportContext {
    pub(crate) source: String,
    pub(crate) title: String,
    pub(crate) date: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct SleepRequest {
    pub(crate) turns: Vec<SleepTurn>,
    pub(crate) custom_model: SleepCustomModel,
    pub(crate) project_tag: Option<String>,
    pub(crate) existing: Vec<SleepExistingMemory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) import_context: Option<SleepImportContext>,
}

#[derive(Deserialize)]
struct SleepMemoryName {
    name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SleepOutcome {
    pub(crate) created: u32,
    pub(crate) updated: u32,
    pub(crate) dropped: u32,
    #[serde(deserialize_with = "memory_names", rename = "memories")]
    pub(crate) memories: Vec<String>,
    /// True when the ledger left nothing to distill — no organ call was made.
    #[serde(default)]
    pub(crate) nothing_new: bool,
    /// Whose hand wrote these memories, when it wasn't the companion's own
    /// model. The organ never sends this — Companion fills it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scribe_note: Option<String>,
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
        let (custom_model, scribe_note) =
            self.resolve_scribe(thread.conversation.companion_id.as_deref())?;

        Ok(Some(PreparedSleep {
            body: SleepRequest {
                turns,
                custom_model,
                project_tag: Some("companion".to_owned()),
                existing: Vec::new(),
                import_context: None,
            },
            bearer,
            agent_id: agent_id.to_owned(),
            fresh_message_ids,
            scribe_note,
        }))
    }

    /// WHICH MODEL holds the distiller's pen for this companion — the voice
    /// resolution /sleep and the history import share, so the two rails can
    /// never diverge. Sync (rusqlite + keyring): call inside spawn_blocking.
    ///
    /// WHICH BRAIN never changes — the memory lands in this companion's own
    /// agent either way. Only the SCRIBE can differ: the organ distills on
    /// an OpenAI-compatible base_url + key, and a Claude Code companion
    /// authenticates with a local login that cannot travel to the server.
    /// So a Claude companion borrows the user's default model to write, and
    /// the outcome says whose hand held the pen — a silent substitution
    /// would be the dishonest part, not the substitution itself.
    pub(crate) fn resolve_scribe(
        &self,
        companion_id: Option<&str>,
    ) -> Result<(SleepCustomModel, Option<String>), AppError> {
        let companion = self.companions.resolve(companion_id)?;
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
        Ok((
            SleepCustomModel {
                base_url: base_url.to_owned(),
                api_key: model.api_key.to_string(),
                model_id: model.model_id,
            },
            scribe_note,
        ))
    }

    /// Which brain an `agent_ref` names — the import worker asks so it can
    /// refuse Muninn targets politely instead of 404ing against a route the
    /// canonical brain does not have.
    pub(crate) fn memory_target(&self, agent_ref: &str) -> Result<MemoryTarget, String> {
        MemoryTarget::resolve(&self.companions, agent_ref)
    }
}

pub(crate) fn load_account_token() -> Result<Option<String>, AppError> {
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
    pub(crate) agent_id: String,
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

    ensure_organ_agent(&name, &description).await
}

/// Find-or-create one agent on the organ roster by name. Extracted from the
/// `ensure_memory_agent` command so backend callers (the call close reporter)
/// can reach a companion's organ memory without a frontend in the loop — the
/// roster round-trip the frontend normally makes before a turn.
pub(crate) async fn ensure_organ_agent(
    name: &str,
    description: &str,
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
        // Reads stay unsigned: the header is authorship, and recall writes no
        // authorship. Sending it here would only change access bookkeeping.
        MemoryTarget::Muninn { channel, .. } => client
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
    let body = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("The recall result could not be read: {error}"))?;

    Ok(match &target {
        MemoryTarget::Organ { .. } => body,
        MemoryTarget::Muninn { channel, .. } => muninn_recall_as_organ(&body, channel),
    })
}

/// Muninn answers `{results: [<flat memory>, ...]}`; the organ answers
/// `{hits: [{memory, score}], vector_leg}` and the frontend reads only the
/// organ's shape. Translate here, where the target is known — the TS contract
/// stays one shape for both products, which is the whole point of the seam.
///
/// Fields Muninn does not carry are filled from what it does: `id` from the
/// name (unique per channel, and the frontend only uses it to dedupe hits
/// riding into one prompt), `agent_id`/`project_tag` from the channel.
fn muninn_recall_as_organ(body: &serde_json::Value, channel: &str) -> serde_json::Value {
    let results = body
        .get("results")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    // A vector leg ran if any hit scored on it. All-zero means the embedder
    // was down and recall fell back to keywords — the same warning the organ's
    // own flag carries.
    let vector_leg = results.iter().any(|hit| {
        hit.get("vec_score")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|score| score > 0.0)
    });

    let hits: Vec<serde_json::Value> = results
        .iter()
        .map(|hit| {
            let text = |key: &str| {
                hit.get(key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let name = text("name");
            serde_json::json!({
                "memory": {
                    "id": name,
                    "agent_id": channel,
                    "name": name,
                    "description": text("description"),
                    "body": text("body"),
                    "mem_type": text("type"),
                    "importance": hit.get("importance").cloned().unwrap_or(serde_json::json!(0.5)),
                    "project_tag": channel,
                    "links": serde_json::Value::Array(Vec::new()),
                    "access_count": hit.get("access_count").cloned().unwrap_or(serde_json::json!(0)),
                    "archived_at": hit.get("archived_at").cloned().unwrap_or(serde_json::Value::Null),
                    "created_at": text("created_at"),
                    "updated_at": text("updated_at"),
                },
                "score": hit.get("score").cloned().unwrap_or(serde_json::json!(0.0)),
            })
        })
        .collect();

    serde_json::json!({ "hits": hits, "vector_leg": vector_leg })
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
        MemoryTarget::Muninn { channel, agent_id } => {
            let request = client
                .post(format!("{MUNINN_BASE}/memories"))
                .json(&muninn_carve_body(payload, channel));
            // Signed when the companion has an identity, unsigned when it does
            // not. An unsigned carve still lands — anonymously — rather than
            // failing, because losing the memory is worse than losing the name.
            match agent_id {
                Some(id) => request.header("X-Agent-Id", id),
                None => request,
            }
        }
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
        MemoryTarget::Muninn { channel, .. } => client
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
pub(crate) async fn fetch_existing_index(bearer: &str, agent_id: &str) -> Vec<SleepExistingMemory> {
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

/// The plain POST — the pre-stream door, the fallback for an organ without
/// /sleep/stream, and the import worker's whole rail (a thousands-of-calls
/// run wants one result per conversation, not a stream per conversation).
pub(crate) async fn sleep_via_post(
    bearer: &str,
    agent_id: &str,
    body: &SleepRequest,
) -> Result<SleepOutcome, String> {
    let response = reqwest::Client::new()
        .post(format!("{MEMORY_ORGAN_BASE}/agents/{agent_id}/sleep"))
        .bearer_auth(bearer)
        .json(body)
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
        None => sleep_via_post(&prepared.bearer, &prepared.agent_id, &prepared.body).await?,
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
    use super::{
        muninn_carve_body, muninn_recall_as_organ, provider_base_url, MemoryTarget, MUNINN_BASE,
    };

    #[test]
    fn every_connected_api_provider_can_power_the_memory_sleep_pass() {
        assert_eq!(
            provider_base_url("together").expect("Together should have a base URL"),
            "https://api.together.ai/v1"
        );
        assert_eq!(
            provider_base_url("openrouter").expect("OpenRouter should have a base URL"),
            "https://openrouter.ai/api/v1"
        );
    }

    /// The frontend reads only the organ's shape. Muninn answering
    /// `{results: [...]}` reached the reflexes as `hits: undefined` and killed
    /// the ambient recall with "undefined is not an object" — live, s509.
    #[test]
    fn a_muninn_recall_arrives_in_the_organs_shape() {
        let body = serde_json::json!({
            "query": "q",
            "mode": "hybrid",
            "results": [{
                "name": "user-moti",
                "description": "one line",
                "body": "the fact",
                "type": "user",
                "importance": 0.9,
                "access_count": 4,
                "archived_at": null,
                "created_at": "2026-08-25T18:00:00+00:00",
                "updated_at": "2026-08-25T18:00:00+00:00",
                "score": 0.42,
                "vec_score": 0.6,
            }],
        });

        let shaped = muninn_recall_as_organ(&body, "canonical");
        let hit = &shaped["hits"][0];

        assert_eq!(shaped["vector_leg"], true);
        assert_eq!(hit["score"], 0.42);
        assert_eq!(hit["memory"]["name"], "user-moti");
        assert_eq!(hit["memory"]["mem_type"], "user", "`type` becomes `mem_type`");
        assert_eq!(hit["memory"]["id"], "user-moti", "the name is the dedupe key");
        assert_eq!(hit["memory"]["agent_id"], "canonical");
        assert_eq!(hit["memory"]["project_tag"], "canonical");
        assert!(hit["memory"]["links"].is_array());
    }

    /// All-zero vector scores mean the embedder was down and recall ran on the
    /// keyword leg alone — the same warning the organ's own flag carries.
    #[test]
    fn a_recall_with_no_vector_scores_reports_a_dead_vector_leg() {
        let body = serde_json::json!({
            "results": [{ "name": "a-thing", "score": 0.1, "vec_score": 0.0 }],
        });

        assert_eq!(muninn_recall_as_organ(&body, "canonical")["vector_leg"], false);
    }

    /// A recall that found nothing must still be a readable empty result, not
    /// a missing key the frontend trips over.
    #[test]
    fn an_empty_recall_is_an_empty_hit_list() {
        let shaped = muninn_recall_as_organ(&serde_json::json!({ "results": [] }), "canonical");

        assert_eq!(shaped["hits"].as_array().map(Vec::len), Some(0));
        assert_eq!(shaped["vector_leg"], false);
    }

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
        let muninn = MemoryTarget::Muninn { channel: "arc".to_owned(), agent_id: None };
        assert_ne!(organ, muninn, "same string, different brain");
    }
}
