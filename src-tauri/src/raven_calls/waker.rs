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

use std::{sync::Arc, time::Duration};

use tauri::{AppHandle, Emitter};

use crate::chat::{drive_turn, AppEventSink, ChatService};

use super::{RavenCallRepository, CALLS_CHANGED_EVENT, CALL_WAKE_EVENT};

/// How often the table is read. Long enough that an idle machine is idle,
/// short enough that an answer feels like a reply rather than a delivery.
const TICK: Duration = Duration::from_secs(4);

/// Wakes started per tick, process-wide. One is deliberate: a woken turn costs
/// a model call, and a burst of them is the failure mode this whole design is
/// built to avoid. A backlog drains one every TICK, which is visible.
const WAKES_PER_TICK: i64 = 1;

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
         2. send_in_call with the same call_id to answer them directly.\n\
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
            service.prepare_woken(&agent_id, text)
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
        if let Err(error) = drive_turn(Arc::clone(chat), prepared, sink).await {
            eprintln!("raven call waker: woken turn failed: {error}");
        }

        // The call moved (or at least was read), so any window showing it
        // should look again. Cheap, and it is the difference between a card
        // that updates itself and one you have to poke.
        let _ = app.emit(CALLS_CHANGED_EVENT, ());
    }

    Ok(())
}
