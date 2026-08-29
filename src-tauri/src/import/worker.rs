// The import loop — the thing that turns a parsed export into memories,
// one conversation at a time, for hours if it has to.
//
// Everything long-lived about a run belongs to the LEDGER, not to this file:
// the worker holds no queue of its own, it asks `next_pending` and marks the
// answer done or failed. That is what makes pause free (stop asking), resume
// free (ask again), and a crash survivable (the row the dead process was
// holding is still pending). The only in-memory state is the parsed export —
// re-read from `source_path` on every (re)start, per the schema 19 note —
// and a per-job signal so pause/cancel land between conversations.
//
// A failed conversation is marked and STEPPED PAST. Hour three of a run must
// never die because conversation 1,204 hit a rate limit; the wizard offers
// one "retry failed" pass at the end instead.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::{
    app_error::AppError,
    chat::repository::{ArchivedImportConversation, ArchivedImportTurn, ChatRepository},
    memory::{
        fetch_existing_index, load_account_token, sleep_via_post, MemoryService, MemoryTarget,
        SleepCustomModel, SleepExistingMemory, SleepImportContext, SleepRequest, SleepTurn,
    },
};

use super::{
    parse_export,
    repository::{ImportJob, ImportJobSnapshot, ImportRepository, JobStatus, NewImportItem},
    ImportedConversation, TurnRole,
};

/// Every progress frame the frontend sees rides this one event: a fresh
/// snapshot after every conversation, plus the title currently distilling.
pub(crate) const IMPORT_PROGRESS_EVENT: &str = "import://progress";

/// The ledger row that stands in for Claude's memories.json — the one part of
/// an export that is already memory-shaped. It rides the queue as a synthetic
/// item so the dedupe, retry and progress stories all hold for it unchanged.
const CLAUDE_MEMORIES_ITEM: &str = "claude:memories.json";

/// How many conversations distill between refreshes of the existing-memory
/// index — the name-reuse leash. Stale by up to ten conversations is fine;
/// the organ upserts by name, so a missed reuse costs a near-duplicate slug,
/// not a wrong memory.
const REFRESH_EXISTING_EVERY: u32 = 10;

/// Per-call size budget, in characters (~60k tokens at ~4 chars each). The
/// corpus says this is rare — the biggest conversation seen is ~72k tokens —
/// so the split path is a guard rail, not a highway.
const MAX_CALL_CHARS: usize = 240_000;

/// How much already-sent text rides along as context when a conversation is
/// split: the tail turns of the previous call, marked context-only so the
/// distiller has footing without re-carving them.
const CONTEXT_TAIL_TURNS: usize = 2;
const CONTEXT_TAIL_MAX_CHARS: usize = 4_000;

/// What pause/cancel look like from inside the loop: a flag checked between
/// conversations. Presence in the control map is also the "a worker is live
/// for this job" marker, which is what stops a double resume.
const SIGNAL_RUN: u8 = 0;
const SIGNAL_PAUSE: u8 = 1;
const SIGNAL_CANCEL: u8 = 2;

pub(crate) struct ImportState {
    service: Arc<ImportService>,
}

impl ImportState {
    pub(crate) fn open(
        database_path: &Path,
        memory: Arc<MemoryService>,
    ) -> Result<Self, AppError> {
        let repository = ImportRepository::open(database_path)?;
        // A `running` row now is a job the previous process died holding.
        repository.park_orphaned_jobs(now_ms())?;
        Ok(Self {
            service: Arc::new(ImportService {
                repository,
                memory,
                database_path: database_path.to_owned(),
                controls: Mutex::new(HashMap::new()),
            }),
        })
    }
}

pub(crate) struct ImportService {
    repository: ImportRepository,
    memory: Arc<MemoryService>,
    /// The app database — the archive pass opens its own ChatRepository on it.
    database_path: PathBuf,
    controls: Mutex<HashMap<String, Arc<AtomicU8>>>,
}

/// One progress frame: the ledger's counts plus what is on the table now.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ImportProgress {
    #[serde(flatten)]
    snapshot: ImportJobSnapshot,
    /// The conversation currently distilling — absent between items and on
    /// the final frame.
    current_title: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartImportOptions {
    /// Fork 3: checked by default in the wizard, so the default here matches.
    #[serde(default = "default_true")]
    pub(crate) include_claude_memories: bool,
    /// The "only last year" middle option: keep conversations created at or
    /// after this stamp. Absent = import everything, which IS the product.
    #[serde(default)]
    pub(crate) since_ms: Option<i64>,
}

fn default_true() -> bool {
    true
}

impl ImportService {
    fn control(&self, job_id: &str) -> Option<Arc<AtomicU8>> {
        self.controls.lock().ok()?.get(job_id).cloned()
    }

    /// Claim the job for a worker. Err = one is already live.
    fn claim(&self, job_id: &str) -> Result<Arc<AtomicU8>, String> {
        let mut controls = self
            .controls
            .lock()
            .map_err(|_| "the import control lock was poisoned".to_owned())?;
        if controls.contains_key(job_id) {
            return Err("This import is already running.".to_owned());
        }
        let signal = Arc::new(AtomicU8::new(SIGNAL_RUN));
        controls.insert(job_id.to_owned(), Arc::clone(&signal));
        Ok(signal)
    }

    fn release(&self, job_id: &str) {
        if let Ok(mut controls) = self.controls.lock() {
            controls.remove(job_id);
        }
    }

    fn snapshot_or_missing(&self, job_id: &str) -> Result<ImportJobSnapshot, String> {
        self.repository
            .snapshot(job_id)
            .map_err(String::from)?
            .ok_or_else(|| "That import no longer exists.".to_owned())
    }
}

#[tauri::command]
pub(crate) async fn start_import(
    app: AppHandle,
    state: State<'_, ImportState>,
    path: String,
    companion_id: String,
    agent_id: String,
    options: StartImportOptions,
) -> Result<ImportJobSnapshot, String> {
    let service = Arc::clone(&state.service);

    let job_id = tauri::async_runtime::spawn_blocking(move || {
        // Fork 2, v1: only organ-target companions import. The canonical
        // Muninn distills as it goes and has no sleep rail to ride.
        if !matches!(
            service.memory.memory_target(&agent_id)?,
            MemoryTarget::Organ { .. }
        ) {
            return Err(
                "This companion remembers through the canonical Muninn, which has no import \
                 rail yet — history import currently works for Semantix-memory companions."
                    .to_owned(),
            );
        }

        let export = parse_export(Path::new(&path))?;
        let latest_ms = export
            .conversations
            .iter()
            .map(|c| c.updated_at_ms.max(c.created_at_ms))
            .max()
            .unwrap_or(0);

        let mut items: Vec<NewImportItem> = export
            .conversations
            .iter()
            .filter(|c| options.since_ms.map_or(true, |since| c.created_at_ms >= since))
            .map(|c| NewImportItem {
                source_id: c.source_id.clone(),
                title: c.title.clone(),
                conversation_at: c.created_at_ms,
                source_updated: c.updated_at_ms.max(c.created_at_ms),
            })
            .collect();
        if options.include_claude_memories && !export.claude_memories.is_empty() {
            // Distilled LAST: memories.json describes the user as of the
            // export, so its era is the newest one in the drop.
            items.push(NewImportItem {
                source_id: CLAUDE_MEMORIES_ITEM.to_owned(),
                title: "Claude's built-in memory of you".to_owned(),
                conversation_at: latest_ms + 1,
                source_updated: latest_ms + 1,
            });
        }
        if items.is_empty() {
            return Err("This export holds no conversations to import.".to_owned());
        }

        let now = now_ms();
        let job = ImportJob {
            id: uuid::Uuid::new_v4().to_string(),
            companion_id,
            agent_ref: agent_id,
            source: export.source,
            source_path: path,
            status: JobStatus::Running,
            error: None,
            include_claude_memories: options.include_claude_memories,
            created_at: now,
            updated_at: now,
        };
        service.repository.create_job(&job, &items).map_err(String::from)?;
        Ok::<_, String>(job.id)
    })
    .await
    .map_err(|error| format!("The import could not start: {error}"))??;

    spawn_worker(app, Arc::clone(&state.service), job_id.clone())?;
    state.service.snapshot_or_missing(&job_id)
}

#[tauri::command]
pub(crate) async fn pause_import(
    state: State<'_, ImportState>,
    job_id: String,
) -> Result<(), String> {
    match state.service.control(&job_id) {
        Some(signal) => {
            signal.store(SIGNAL_PAUSE, Ordering::Relaxed);
            Ok(())
        }
        None => Err("That import is not running.".to_owned()),
    }
}

#[tauri::command]
pub(crate) async fn resume_import(
    app: AppHandle,
    state: State<'_, ImportState>,
    job_id: String,
) -> Result<ImportJobSnapshot, String> {
    let snapshot = state.service.snapshot_or_missing(&job_id)?;
    match snapshot.job.status {
        JobStatus::Paused | JobStatus::Failed => {}
        JobStatus::Running => return Err("This import is already running.".to_owned()),
        JobStatus::Done => return Err("This import already finished.".to_owned()),
        JobStatus::Cancelled => return Err("This import was cancelled.".to_owned()),
    }
    spawn_worker(app, Arc::clone(&state.service), job_id.clone())?;
    state.service.snapshot_or_missing(&job_id)
}

#[tauri::command]
pub(crate) async fn cancel_import(
    state: State<'_, ImportState>,
    job_id: String,
) -> Result<(), String> {
    if let Some(signal) = state.service.control(&job_id) {
        // A live worker parks the job itself once the current conversation
        // lands — cancelling mid-distill would waste the call it already paid.
        signal.store(SIGNAL_CANCEL, Ordering::Relaxed);
        return Ok(());
    }
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || {
        service
            .repository
            .set_job_status(&job_id, JobStatus::Cancelled, None, now_ms())
            .map_err(String::from)
    })
    .await
    .map_err(|error| format!("The cancel failed: {error}"))?
}

/// The one "retry failed" pass at the end of a run: failures rejoin the queue
/// and, if no worker is live, one is started to drain them.
#[tauri::command]
pub(crate) async fn retry_failed_import(
    app: AppHandle,
    state: State<'_, ImportState>,
    job_id: String,
) -> Result<ImportJobSnapshot, String> {
    let service = Arc::clone(&state.service);
    let requeue_job_id = job_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.repository.requeue_failed(&requeue_job_id).map_err(String::from)
    })
    .await
    .map_err(|error| format!("The retry failed: {error}"))??;

    if state.service.control(&job_id).is_none() {
        spawn_worker(app, Arc::clone(&state.service), job_id.clone())?;
    }
    state.service.snapshot_or_missing(&job_id)
}

#[tauri::command]
pub(crate) async fn list_import_jobs(
    state: State<'_, ImportState>,
) -> Result<Vec<ImportJobSnapshot>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || {
        service.repository.list_snapshots().map_err(String::from)
    })
    .await
    .map_err(|error| format!("The import list failed: {error}"))?
}

/// Claim the job, mark it running, and hand it to a background task. The
/// claim happens BEFORE the spawn so a double resume fails fast in the
/// command, not as two workers racing over one queue.
fn spawn_worker(app: AppHandle, service: Arc<ImportService>, job_id: String) -> Result<(), String> {
    let signal = service.claim(&job_id)?;
    tauri::async_runtime::spawn(async move {
        let final_status = run_job(&app, &service, &job_id, &signal).await;
        if let Err(error) = &final_status {
            let _ = service.repository.set_job_status(
                &job_id,
                JobStatus::Failed,
                Some(error),
                now_ms(),
            );
            eprintln!("[import] job {job_id} failed: {error}");
        }
        service.release(&job_id);
        emit_progress(&app, &service, &job_id, None);
    });
    Ok(())
}

/// Everything a run needs in hand before the first conversation distills —
/// gathered in one spawn_blocking pass (rusqlite + keyring are sync).
struct PreparedRun {
    job: ImportJob,
    conversations: HashMap<String, ImportedConversation>,
    claude_memories: Vec<String>,
    bearer: String,
    custom_model: SleepCustomModel,
}

async fn run_job(
    app: &AppHandle,
    service: &Arc<ImportService>,
    job_id: &str,
    signal: &Arc<AtomicU8>,
) -> Result<(), String> {
    let prepared = {
        let service = Arc::clone(service);
        let job_id = job_id.to_owned();
        tauri::async_runtime::spawn_blocking(move || prepare_run(&service, &job_id))
            .await
            .map_err(|error| format!("The import could not prepare: {error}"))??
    };
    let prepared = Arc::new(prepared);
    service
        .repository
        .set_job_status(job_id, JobStatus::Running, None, now_ms())
        .map_err(String::from)?;

    // THE ARCHIVE PASS — every conversation this job covers lands in the chat
    // database as a real (archived, hidden) conversation BEFORE the first
    // distillation call: word-for-word search works seconds after the drop,
    // while memories trickle in behind it for hours. Runs on every (re)start
    // of the job and upserts by deterministic id, so a resume — including one
    // of a job that predates this pass — backfills what is missing and skips
    // what is already filed. Fail-open: a dead archive pass is logged and the
    // distiller still runs; the next resume gets another try.
    {
        emit_progress(app, service, job_id, Some("Filing your conversations…".to_owned()));
        let service = Arc::clone(service);
        let prepared = Arc::clone(&prepared);
        let job_id = job_id.to_owned();
        let filed = tauri::async_runtime::spawn_blocking(move || {
            archive_job_conversations(&service, &prepared, &job_id)
        })
        .await
        .map_err(|error| format!("The archive pass could not run: {error}"))?;
        match filed {
            Ok((added, refreshed)) if added + refreshed > 0 => {
                println!("[import] archived {added} new + {refreshed} refreshed conversation(s)");
            }
            Ok(_) => {}
            Err(error) => eprintln!("[import] archive pass failed (distilling anyway): {error}"),
        }
    }

    let mut existing: Vec<SleepExistingMemory> = Vec::new();
    let mut distilled_since_refresh = REFRESH_EXISTING_EVERY;

    let parked_as = loop {
        match signal.load(Ordering::Relaxed) {
            SIGNAL_PAUSE => break JobStatus::Paused,
            SIGNAL_CANCEL => break JobStatus::Cancelled,
            _ => {}
        }

        let item = {
            let service = Arc::clone(service);
            let job_id = job_id.to_owned();
            tauri::async_runtime::spawn_blocking(move || service.repository.next_pending(&job_id))
                .await
                .map_err(|error| format!("The import queue failed: {error}"))?
                .map_err(String::from)?
        };
        let Some(item) = item else { break JobStatus::Done };

        if distilled_since_refresh >= REFRESH_EXISTING_EVERY {
            existing = fetch_existing_index(&prepared.bearer, &prepared.job.agent_ref).await;
            distilled_since_refresh = 0;
        }
        emit_progress(app, service, job_id, Some(item.title.clone()));

        let outcome = distill_item(&prepared, &existing, &item).await;
        {
            let service = Arc::clone(service);
            let job_id = job_id.to_owned();
            let source_id = item.source_id.clone();
            tauri::async_runtime::spawn_blocking(move || match outcome {
                Ok((created, updated)) => service
                    .repository
                    .mark_item_done(&job_id, &source_id, created, updated, now_ms()),
                Err(error) => {
                    eprintln!("[import] {source_id} failed: {error}");
                    service
                        .repository
                        .mark_item_failed(&job_id, &source_id, &error, now_ms())
                }
            })
            .await
            .map_err(|error| format!("The import ledger failed: {error}"))?
            .map_err(String::from)?;
        }
        distilled_since_refresh += 1;
        emit_progress(app, service, job_id, None);
    };

    service
        .repository
        .set_job_status(job_id, parked_as, None, now_ms())
        .map_err(String::from)?;
    Ok(())
}

/// File this job's conversations into the chat archive as real rows. Scope is
/// the job's LEDGER items (skipped ones included — already-distilled is not
/// already-archived), joined back to the re-parsed export by source_id; the
/// synthetic memories.json item has no conversation and is left out.
fn archive_job_conversations(
    service: &ImportService,
    prepared: &PreparedRun,
    job_id: &str,
) -> Result<(usize, usize), String> {
    let source_ids = service.repository.item_source_ids(job_id).map_err(String::from)?;
    let source = prepared.job.source.as_str();
    let records: Vec<ArchivedImportConversation<'_>> = source_ids
        .iter()
        .filter(|source_id| source_id.as_str() != CLAUDE_MEMORIES_ITEM)
        .filter_map(|source_id| prepared.conversations.get(source_id))
        .map(|conversation| ArchivedImportConversation {
            id: format!("import:{source}:{}", conversation.source_id),
            title: &conversation.title,
            companion_id: &prepared.job.companion_id,
            source,
            created_at: conversation.created_at_ms,
            updated_at: conversation.updated_at_ms.max(conversation.created_at_ms),
            turns: conversation
                .turns
                .iter()
                .map(|turn| ArchivedImportTurn {
                    role: match turn.role {
                        TurnRole::User => "user",
                        TurnRole::Assistant => "assistant",
                    },
                    text: &turn.text,
                })
                .collect(),
        })
        .collect();
    let chat = ChatRepository::open(&service.database_path).map_err(String::from)?;
    chat.archive_imported_conversations(&records, now_ms())
        .map_err(String::from)
}

fn prepare_run(service: &ImportService, job_id: &str) -> Result<PreparedRun, String> {
    let job = service
        .repository
        .snapshot(job_id)
        .map_err(String::from)?
        .ok_or_else(|| "That import no longer exists.".to_owned())?
        .job;
    let bearer = load_account_token()
        .map_err(String::from)?
        .ok_or_else(|| "Connect your Semantix account before importing.".to_owned())?;
    // The same voice resolution /sleep uses — a Claude Code companion borrows
    // the default model here exactly as it does there.
    let (custom_model, _scribe_note) = service
        .memory
        .resolve_scribe(Some(&job.companion_id))
        .map_err(String::from)?;

    // Re-read from the source every (re)start — the ledger holds titles and
    // stamps, never conversation text.
    let export = parse_export(Path::new(&job.source_path)).map_err(|error| {
        format!(
            "The export could not be re-read from {} — put it back (or start a new import \
             with its new location) and resume. {error}",
            job.source_path
        )
    })?;
    let conversations = export
        .conversations
        .into_iter()
        .map(|conversation| (conversation.source_id.clone(), conversation))
        .collect();

    Ok(PreparedRun {
        job,
        conversations,
        claude_memories: export.claude_memories,
        bearer,
        custom_model,
    })
}

/// One ledger item → one distilled conversation (or a handful of calls when
/// it is oversized). Returns (memories created, memories updated).
async fn distill_item(
    prepared: &PreparedRun,
    existing: &[SleepExistingMemory],
    item: &super::repository::PendingItem,
) -> Result<(i64, i64), String> {
    let (turns, context_source) = if item.source_id == CLAUDE_MEMORIES_ITEM {
        let turns = prepared
            .claude_memories
            .iter()
            .map(|blob| SleepTurn { role: "user", text: blob.clone(), context: false })
            .collect::<Vec<_>>();
        if turns.is_empty() {
            return Err("The export no longer carries a memories.json.".to_owned());
        }
        (turns, "claude-memories")
    } else {
        let conversation = prepared.conversations.get(&item.source_id).ok_or_else(|| {
            "This conversation is not in the export any more — it may be a different \
             export file at the same path."
                .to_owned()
        })?;
        let turns = conversation
            .turns
            .iter()
            .map(|turn| SleepTurn {
                role: match turn.role {
                    TurnRole::User => "user",
                    TurnRole::Assistant => "assistant",
                },
                text: turn.text.clone(),
                context: false,
            })
            .collect::<Vec<_>>();
        (turns, prepared.job.source.as_str())
    };

    let import_context = SleepImportContext {
        source: context_source.to_owned(),
        title: item.title.clone(),
        date: date_from_epoch_ms(item.conversation_at),
    };

    let mut created = 0i64;
    let mut updated = 0i64;
    for turns in split_into_calls(turns) {
        let body = SleepRequest {
            turns,
            custom_model: SleepCustomModel {
                base_url: prepared.custom_model.base_url.clone(),
                api_key: prepared.custom_model.api_key.clone(),
                model_id: prepared.custom_model.model_id.clone(),
            },
            project_tag: Some("companion".to_owned()),
            existing: existing.to_vec(),
            import_context: Some(SleepImportContext {
                source: import_context.source.clone(),
                title: import_context.title.clone(),
                date: import_context.date.clone(),
            }),
        };
        let outcome = sleep_via_post(&prepared.bearer, &prepared.job.agent_ref, &body).await?;
        created += i64::from(outcome.created);
        updated += i64::from(outcome.updated);
    }
    Ok((created, updated))
}

/// Split a conversation into per-call turn lists under the size budget. Almost
/// every conversation fits in one; an oversized one continues in the next call
/// with the previous call's tail riding along as context-only footing.
fn split_into_calls(turns: Vec<SleepTurn>) -> Vec<Vec<SleepTurn>> {
    let total: usize = turns.iter().map(|turn| turn.text.len()).sum();
    if total <= MAX_CALL_CHARS {
        return vec![turns];
    }

    let mut calls: Vec<Vec<SleepTurn>> = Vec::new();
    let mut call: Vec<SleepTurn> = Vec::new();
    let mut call_chars = 0usize;
    for turn in turns {
        // A single turn over the whole budget keeps its head; the truncation
        // costs detail, never the conversation.
        let turn = if turn.text.len() > MAX_CALL_CHARS {
            SleepTurn {
                role: turn.role,
                text: truncate_at_char_boundary(&turn.text, MAX_CALL_CHARS).to_owned(),
                context: turn.context,
            }
        } else {
            turn
        };
        if !call.is_empty() && call_chars + turn.text.len() > MAX_CALL_CHARS {
            let tail = context_tail(&call);
            calls.push(std::mem::take(&mut call));
            call_chars = tail.iter().map(|t| t.text.len()).sum();
            call = tail;
        }
        call_chars += turn.text.len();
        call.push(turn);
    }
    // A trailing context-only tail carries nothing worth a model call.
    if call.iter().any(|turn| !turn.context) {
        calls.push(call);
    }
    calls
}

fn context_tail(call: &[SleepTurn]) -> Vec<SleepTurn> {
    call.iter()
        .rev()
        .take(CONTEXT_TAIL_TURNS)
        .map(|turn| SleepTurn {
            role: turn.role,
            text: truncate_at_char_boundary(&turn.text, CONTEXT_TAIL_MAX_CHARS).to_owned(),
            context: true,
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn emit_progress(
    app: &AppHandle,
    service: &Arc<ImportService>,
    job_id: &str,
    current_title: Option<String>,
) {
    let Ok(Some(snapshot)) = service.repository.snapshot(job_id) else { return };
    let _ = app.emit(IMPORT_PROGRESS_EVENT, ImportProgress { snapshot, current_title });
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Epoch ms → "YYYY-MM-DD", the era stamp the import prompt reads. Hinnant's
/// `civil_from_days`, the inverse of the parser's `days_from_civil`.
fn date_from_epoch_ms(ms: i64) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::{
        date_from_epoch_ms, split_into_calls, truncate_at_char_boundary, MAX_CALL_CHARS,
    };
    use crate::memory::SleepTurn;

    fn turn(role: &'static str, chars: usize) -> SleepTurn {
        SleepTurn { role, text: "x".repeat(chars), context: false }
    }

    #[test]
    fn era_stamps_round_trip_through_the_civil_calendar() {
        assert_eq!(date_from_epoch_ms(0), None, "an undated conversation has no era");
        assert_eq!(
            date_from_epoch_ms(1_689_810_797_167).as_deref(),
            Some("2023-07-19"),
            "the first conversation of the real Claude corpus"
        );
        assert_eq!(date_from_epoch_ms(86_400_000).as_deref(), Some("1970-01-02"));
        assert_eq!(date_from_epoch_ms(1_767_348_000_500).as_deref(), Some("2026-01-02"));
    }

    #[test]
    fn a_normal_conversation_is_one_call() {
        let calls = split_into_calls(vec![turn("user", 100), turn("assistant", 200)]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 2);
    }

    /// The oversize path: later calls open with the previous call's tail as
    /// context-only footing, and every call stays under the budget.
    #[test]
    fn an_oversized_conversation_splits_with_a_context_tail() {
        let big = MAX_CALL_CHARS / 2;
        let calls = split_into_calls(vec![
            turn("user", big),
            turn("assistant", big),
            turn("user", big),
        ]);

        assert_eq!(calls.len(), 2);
        assert!(calls[1][0].context, "the next call opens on carried footing");
        assert!(!calls[1].last().unwrap().context, "and ends on fresh text");
        for call in &calls {
            let fresh: usize =
                call.iter().filter(|t| !t.context).map(|t| t.text.len()).sum();
            assert!(fresh <= MAX_CALL_CHARS);
        }
    }

    #[test]
    fn one_giant_turn_is_truncated_not_fatal() {
        let calls = split_into_calls(vec![turn("user", MAX_CALL_CHARS * 2)]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0].text.len(), MAX_CALL_CHARS);
    }

    #[test]
    fn truncation_respects_multibyte_boundaries() {
        let text = "אב".repeat(10); // 4 bytes per pair
        let cut = truncate_at_char_boundary(&text, 5);
        assert_eq!(cut, "אב"); // 5 lands mid-א of the second pair → backs off to 4
        assert!(cut.len() <= 5);
    }
}
