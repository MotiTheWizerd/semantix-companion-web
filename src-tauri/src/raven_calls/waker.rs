//! THE WAKER — the thing that gives a companion a turn nobody asked for.
//!
//! Everything before this was plumbing that still needed a person: a call
//! could be placed, stored, scoped and rendered, and it still sat in the
//! database until someone pressed enter. This is the loop that reads the
//! table and starts a turn on its own.
//!
//! ⚑ WHY A POLL AND NOT A SIGNAL. A call is written by a tool running inside
//! another turn, so an in-process notify would work — and would silently do
//! nothing the day a call arrives from anywhere else (another window, a
//! future sync, a repair script). Reading the table is the version that
//! cannot be bypassed by a new writer, and at one query every few seconds
//! over an index it costs nothing worth optimising.
//!
//! WHAT KEEPS THIS FROM RUNNING AWAY, in order of how much each one matters:
//!   1. The 5-message cap closes a call, so a two-agent exchange terminates
//!      whether or not anyone intended it to.
//!   2. The wake guard (schema 15) fires at most one wake per thing said, so
//!      a companion that declines to answer stays quiet instead of being
//!      asked again every tick.
//!   3. One wake per tick, process-wide, so a backlog drains at a visible
//!      pace rather than starting a dozen model calls at once.

use std::{collections::HashMap, sync::Arc, time::Duration};

use tauri::{AppHandle, Emitter};

use crate::{
    chat::{drive_turn, AppEventSink, ChatEvent, ChatService, CHAT_EVENT},
    companions::Companion,
    memory,
};

use super::{record, RavenCall, RavenCallRepository, CALLS_CHANGED_EVENT, CALL_WAKE_EVENT};

/// How often the table is read. Long enough that an idle machine is idle,
/// short enough that an answer feels like a reply rather than a delivery.
const TICK: Duration = Duration::from_secs(4);

/// Wakes started per tick, process-wide. One is deliberate: a woken turn costs
/// a model call, and a burst of them is the failure mode this whole design is
/// built to avoid. A backlog drains one every TICK, which is visible.
const WAKES_PER_TICK: i64 = 1;

/// Close records delivered per tick — same pacing, same reasoning: a report
/// drives a model call of its own.
const REPORTS_PER_TICK: i64 = 1;

/// Turns loaded to build one record. A call cannot exceed its message cap, so
/// this only bites if that cap is raised — bounded on principle, because the
/// message table is the one that grows.
const RECORD_MESSAGE_LIMIT: i64 = 50;

/// What a woken companion is told. It is a `system` message in its own thread,
/// so it must read as an event that happened rather than as a person speaking.
///
/// It names the tools instead of summarising the call, on purpose: handing over
/// the text here would let the companion answer without ever calling
/// `read_call`, and then the call's own record would not show it had been read.
fn notice(call_id: &str) -> String {
    format!(
        "You have been woken to answer a call. This is not a conversation with your user — \
         they did not send this and are not reading it. Another agent is holding the line.\n\
         \n\
         Do this now, in this turn:\n\
         1. read_call with call_id \"{call_id}\" to see what they said.\n\
         2. send_in_call with the same call_id to answer them directly. If your answer \
         completes the exchange — the question is answered, the errand is done — pass \
         final: true with it to hang up: your last word still reaches them through the \
         call record, and neither of you is woken again just to trade goodbyes.\n\
         \n\
         Write your reply TO THE OTHER AGENT, in the second person, as one side of a \
         conversation between the two of you. Do not describe what you are doing, do not \
         address your user, and do not answer here in the chat — text you write outside \
         send_in_call reaches nobody. The call is the only way your answer arrives.\n\
         If the call genuinely needs no reply, call read_call anyway and then stop."
    )
}

/// Start the waker. Runs for the life of the process.
pub(crate) fn spawn(app: AppHandle, calls: Arc<RavenCallRepository>, chat: Arc<ChatService>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if let Err(error) = tick(&app, &calls, &chat).await {
                // A failing tick must never end the loop — a transient
                // database or provider error would otherwise silently retire
                // the whole feature until the app restarts.
                eprintln!("raven call waker: {error}");
            }
        }
    });
}

async fn tick(
    app: &AppHandle,
    calls: &Arc<RavenCallRepository>,
    chat: &Arc<ChatService>,
) -> Result<(), String> {
    let repository = Arc::clone(calls);
    let pending = tauri::async_runtime::spawn_blocking(move || {
        repository.calls_awaiting_wake(WAKES_PER_TICK)
    })
    .await
    .map_err(|error| format!("the wake scan failed: {error}"))?
    .map_err(|error| error.to_string())?;

    for wake in pending {
        // ⚑ MARKED BEFORE THE TURN RUNS, NOT AFTER. A turn that fails, hangs
        // or panics still counts as "we tried" — otherwise a companion whose
        // model is erroring gets woken again every tick for as long as the
        // error lasts, which is exactly when you least want a retry storm.
        let repository = Arc::clone(calls);
        let (call_id, message_id) = (wake.call_id.clone(), wake.message_id.clone());
        tauri::async_runtime::spawn_blocking(move || repository.mark_woken(&call_id, &message_id))
            .await
            .map_err(|error| format!("the wake guard failed: {error}"))?
            .map_err(|error| error.to_string())?;

        let service = Arc::clone(chat);
        let agent_id = wake.agent_id.clone();
        let text = notice(&wake.call_id);
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            service.prepare_woken(&agent_id, text, None)
        })
        .await
        .map_err(|error| format!("the woken submission failed: {error}"))?;

        let prepared = match prepared {
            Ok(prepared) => prepared,
            // A companion that no longer exists, or one whose model is not
            // configured, is not an error worth retrying — the guard above
            // already means we will not come back to this message.
            Err(error) => {
                eprintln!("raven call waker: could not wake {}: {error}", wake.agent_id);
                // The guard above is already set, so this call will never be
                // tried again on its own — the one thing that must not happen
                // is the UI still saying "ringing". The changed event makes
                // every window re-read and find the guard on the newest turn:
                // the silence becomes "no answer", which a person can retry.
                let _ = app.emit(CALLS_CHANGED_EVENT, ());
                continue;
            }
        };

        // Armed here, disarmed by the CALLS_CHANGED_EVENT below — which runs
        // whether the turn succeeds or fails, so the pairing cannot leak.
        let _ = app.emit(
            CALL_WAKE_EVENT,
            serde_json::json!({ "callId": wake.call_id, "agentId": wake.agent_id }),
        );

        let sink = Arc::new(AppEventSink::new(app.clone()));
        if let Err(error) = drive_turn(Arc::clone(chat), prepared, sink, None).await {
            eprintln!("raven call waker: woken turn failed: {error}");
        }

        // The call moved (or at least was read), so any window showing it
        // should look again. Cheap, and it is the difference between a card
        // that updates itself and one you have to poke.
        let _ = app.emit(CALLS_CHANGED_EVENT, ());
    }

    report_closed_calls(app, calls, chat).await
}

/// The second half of the tick: calls that have CLOSED since the last look get
/// their record delivered — a transcript into each participant's thread, a
/// carve into each participant's long-term memory, and one woken turn for the
/// initiator so it can tell its user what came of the call.
///
/// ⚑ WHY ANY OF THIS EXISTS. Everything a companion learns during a call lives
/// in woken turns whose tool results are never resent, and `list_calls` hides
/// closed calls — so the moment a call ended, BOTH participants forgot it ever
/// happened (s502). Worse, the turn that fills a call wakes nobody, so the
/// last word of every full call was never seen by its addressee at all.
async fn report_closed_calls(
    app: &AppHandle,
    calls: &Arc<RavenCallRepository>,
    chat: &Arc<ChatService>,
) -> Result<(), String> {
    let repository = Arc::clone(calls);
    let pending = tauri::async_runtime::spawn_blocking(move || {
        repository.calls_needing_close_report(REPORTS_PER_TICK)
    })
    .await
    .map_err(|error| format!("the close-report scan failed: {error}"))?
    .map_err(|error| error.to_string())?;

    for call in pending {
        // ⚑ MARKED BEFORE THE DELIVERY RUNS, NOT AFTER — the same law as
        // `mark_woken` above, for the same reason: a report whose wake fails
        // must still count as "we tried", or an erroring model is handed the
        // same transcript every four seconds for as long as the error lasts.
        let repository = Arc::clone(calls);
        let call_id = call.id.clone();
        tauri::async_runtime::spawn_blocking(move || repository.mark_close_reported(&call_id))
            .await
            .map_err(|error| format!("the close-report guard failed: {error}"))?
            .map_err(|error| error.to_string())?;

        if let Err(error) = deliver_close_record(app, calls, chat, &call).await {
            eprintln!("raven call close record: {error}");
        }
        let _ = app.emit(CALLS_CHANGED_EVENT, ());
    }

    Ok(())
}

/// Deliver ONE closed call's record. Every step is best-effort past the one
/// before it: the thread copies land first (plain database writes), the
/// carves next (an unreachable memory organ must not eat the record), and the
/// initiator's woken turn last — with a persist-only fallback, so even a dead
/// model leaves the transcript in the thread the call was born from.
async fn deliver_close_record(
    app: &AppHandle,
    calls: &Arc<RavenCallRepository>,
    chat: &Arc<ChatService>,
    call: &RavenCall,
) -> Result<(), String> {
    let repository = Arc::clone(calls);
    let call_id = call.id.clone();
    let messages = tauri::async_runtime::spawn_blocking(move || {
        repository.messages(&call_id, RECORD_MESSAGE_LIMIT)
    })
    .await
    .map_err(|error| format!("the record read failed: {error}"))?
    .map_err(|error| error.to_string())?;
    if messages.is_empty() {
        return Ok(());
    }

    let initiator_id = call.initiator_agent_id.clone();
    let other_id = messages
        .iter()
        .flat_map(|message| [message.from_agent_id.clone(), message.to_agent_id.clone()])
        .find(|id| *id != initiator_id);

    // `None` = not a companion on this machine any more; the record still
    // renders with a stable label, it just has no thread or memory to land in.
    let initiator = profile(chat, &initiator_id).await?;
    let other = match &other_id {
        Some(id) => profile(chat, id).await?,
        None => None,
    };

    let mut names = HashMap::new();
    if let Some(companion) = &initiator {
        if let Some(name) = &companion.name {
            names.insert(initiator_id.clone(), name.clone());
        }
    }
    if let (Some(id), Some(companion)) = (&other_id, &other) {
        if let Some(name) = &companion.name {
            names.insert(id.clone(), name.clone());
        }
    }
    let initiator_name = record::display_name(&names, &initiator_id);
    let other_name = other_id
        .as_deref()
        .map(|id| record::display_name(&names, id))
        .unwrap_or_else(|| initiator_name.clone());

    let repository = Arc::clone(calls);
    let created_at = call.created_at;
    let stamp = tauri::async_runtime::spawn_blocking(move || repository.local_datetime(created_at))
        .await
        .map_err(|error| format!("the record stamp failed: {error}"))?
        .map_err(|error| error.to_string())?;

    let transcript = record::render_transcript(call, &messages, &names, &stamp);

    // The other side's thread copy — persist-only, no turn driven. Their next
    // conversation simply knows the call happened, because `system` rows ride
    // every later request.
    if let (Some(id), Some(_)) = (&other_id, &other) {
        record_into_thread(app, chat, id, None, record::thread_record(&transcript)).await;
    }

    // Both carves, each into that companion's OWN memory, each naming the
    // OTHER side. Best-effort: a missing Semantix account or a dead organ is
    // logged and stepped past — the thread copies above already hold the
    // record.
    let mut initiator_carved: Option<String> = None;
    let opener = messages[0].body.clone();
    let sides = [
        (initiator.as_ref(), other_name.clone()),
        (other.as_ref(), initiator_name.clone()),
    ];
    for (companion, counterpart) in sides {
        let Some(companion) = companion else { continue };
        let payload = record::carve_payload(call, &counterpart, &opener, &transcript, &stamp);
        match carve(companion, &payload).await {
            Ok(()) => {
                if companion.id == call.initiator_agent_id {
                    initiator_carved = payload["name"].as_str().map(str::to_owned);
                }
            }
            Err(error) => eprintln!(
                "raven call close record: carve for {} failed: {error}",
                companion.id
            ),
        }
    }

    // The initiator's report turn — the companion comes back from the phone
    // and tells its user what happened, in the conversation the call was born
    // from. The notice only claims a carve that actually landed.
    if initiator.is_some() {
        let carve_line = match &initiator_carved {
            Some(name) => format!("A copy was carved into your long-term memory as [{name}]."),
            None => "It could not be carved into your long-term memory this time, so this \
                     thread holds your only copy."
                .to_owned(),
        };
        let notice = record::close_notice(&transcript, &carve_line);
        let service = Arc::clone(chat);
        let agent_id = initiator_id.clone();
        let root = call.root_conversation_id.clone();
        let prepared = tauri::async_runtime::spawn_blocking(move || {
            service.prepare_woken(&agent_id, notice, root)
        })
        .await
        .map_err(|error| format!("the report submission failed: {error}"))?;

        match prepared {
            Ok(prepared) => {
                let sink = Arc::new(AppEventSink::new(app.clone()));
                if let Err(error) = drive_turn(Arc::clone(chat), prepared, sink, None).await {
                    eprintln!("raven call close record: report turn failed: {error}");
                }
            }
            // The record must not die with the wake — a companion whose model
            // is unconfigured still gets the transcript, written plainly.
            Err(error) => {
                eprintln!("raven call close record: could not wake {initiator_id}: {error}");
                record_into_thread(
                    app,
                    chat,
                    &initiator_id,
                    call.root_conversation_id.as_deref(),
                    record::thread_record(&transcript),
                )
                .await;
            }
        }
    }

    Ok(())
}

/// One companion's full row, or `None` for an id this machine cannot place.
async fn profile(
    chat: &Arc<ChatService>,
    companion_id: &str,
) -> Result<Option<Companion>, String> {
    let service = Arc::clone(chat);
    let id = companion_id.to_owned();
    tauri::async_runtime::spawn_blocking(move || service.companion_profile(&id))
        .await
        .map_err(|error| format!("the companion lookup failed: {error}"))?
        .map_err(|error| error.to_string())
}

/// Persist one system record into a companion's thread and tell every window.
/// Failures are logged, never propagated — one thread copy failing must not
/// cost the other side its own.
async fn record_into_thread(
    app: &AppHandle,
    chat: &Arc<ChatService>,
    companion_id: &str,
    preferred_conversation: Option<&str>,
    content: String,
) {
    let service = Arc::clone(chat);
    let id = companion_id.to_owned();
    let preferred = preferred_conversation.map(str::to_owned);
    let written = tauri::async_runtime::spawn_blocking(move || {
        service.record_system_message(&id, preferred.as_deref(), &content)
    })
    .await;
    match written {
        Ok(Ok(accepted)) => {
            // The same event shape a woken turn opens with, so a window
            // already showing the thread folds the record in live.
            let _ = app.emit(
                CHAT_EVENT,
                ChatEvent::Accepted {
                    conversation: accepted.conversation,
                    message: accepted.message,
                },
            );
        }
        Ok(Err(error)) => {
            eprintln!("raven call close record: thread copy for {companion_id} failed: {error}");
        }
        Err(error) => {
            eprintln!("raven call close record: thread copy task failed: {error}");
        }
    }
}

/// Carve one call record into one companion's own memory, wherever that
/// memory lives — the organ needs its roster round-trip first, Muninn is
/// addressed by channel directly. Same doors as the model's own `carve_memory`.
async fn carve(companion: &Companion, payload: &serde_json::Value) -> Result<(), String> {
    let target = if companion.is_origin {
        memory::MemoryTarget::Muninn {
            channel: companion.memory_agent_name.clone(),
            agent_id: companion.origin_agent_id.clone(),
        }
    } else {
        let agent = memory::ensure_organ_agent(
            &companion.memory_agent_name,
            "Private memory of a Semantix companion",
        )
        .await?;
        memory::MemoryTarget::Organ { agent_id: agent.agent_id }
    };
    memory::write_memory(&target, payload).await.map(|_| ())
}
