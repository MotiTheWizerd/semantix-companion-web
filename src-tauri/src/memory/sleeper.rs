//! The sleeper — memory that carves itself.
//!
//! /sleep (s478) distils a conversation into memory, and since s487 only the
//! turns nobody has slept on yet. The two problems Moti named at s539 — "the
//! user has to remember to do it" and "on a long chat it takes forever" —
//! were one problem: nobody calls it. A chat slept every dozen turns never
//! has a long pass, because every pass is a dozen turns.
//!
//! The rule is the web hall's walker (s414), brought home: a conversation is
//! RIPE when it holds a dozen fresh turns, or a few and three minutes of
//! silence. Whether any of it is worth keeping is the distiller's call — an
//! empty answer is common and correct. This runs in the backend, where the
//! ledger, the vault and the roster live, so it works whichever tab is open —
//! and it never takes the keyboard: the composer is not locked, the outcome
//! arrives as an app-wide event.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::{run_sleep_pass, MemoryService, MemoryTarget, SleepOutcome};

/// Fresh turns that make a conversation ripe on the spot.
const RIPE_TURNS: usize = 12;
/// Fresh turns enough to sleep on once the thread has gone quiet.
const IDLE_MIN_TURNS: usize = 4;
/// How long a thread must be silent before the quiet rule fires.
const IDLE_AFTER: Duration = Duration::from_secs(180);
/// After a failed pass, leave the thread alone this long — an organ that is
/// down does not need to hear about it every turn.
const BACKOFF_AFTER_ERROR: Duration = Duration::from_secs(600);

/// App-wide: the sleeper finished a pass on its own.
pub(crate) const SLEPT_EVENT: &str = "memory://slept";

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum SleptEvent {
    /// A pass ran and landed. `created` may be 0 — the distiller read the
    /// turns and kept nothing, which is a result, not a failure.
    Carved {
        #[serde(rename = "conversationId")]
        conversation_id: String,
        created: u32,
        updated: u32,
        dropped: u32,
        memories: Vec<String>,
        #[serde(rename = "scribeNote", skip_serializing_if = "Option::is_none")]
        scribe_note: Option<String>,
    },
    Failed {
        #[serde(rename = "conversationId")]
        conversation_id: String,
        message: String,
    },
}

#[derive(Default)]
struct Watch {
    /// Bumped when a turn starts and again when it lands; a quiet-rule timer
    /// that wakes to a different generation was overtaken and stands down.
    generation: u64,
    /// A pass — the sleeper's or a manual /sleep — is on this thread now.
    running: bool,
    backoff_until: Option<Instant>,
}

pub(crate) struct Sleeper {
    app: AppHandle,
    service: Arc<MemoryService>,
    watches: Mutex<HashMap<String, Watch>>,
}

/// Held while a pass runs; dropping it releases the thread.
pub(crate) struct PassGuard {
    sleeper: Arc<Sleeper>,
    conversation_id: String,
}

impl Drop for PassGuard {
    fn drop(&mut self) {
        if let Ok(mut watches) = self.sleeper.watches.lock() {
            if let Some(watch) = watches.get_mut(&self.conversation_id) {
                watch.running = false;
            }
        }
    }
}

impl Sleeper {
    pub(crate) fn new(app: AppHandle, service: Arc<MemoryService>) -> Arc<Self> {
        Arc::new(Self {
            app,
            service,
            watches: Mutex::new(HashMap::new()),
        })
    }

    /// Claim the thread for one pass. `None` = a pass is already running on
    /// it; the caller does not get to start another.
    pub(crate) fn begin(self: &Arc<Self>, conversation_id: &str) -> Option<PassGuard> {
        let mut watches = self.watches.lock().ok()?;
        let watch = watches.entry(conversation_id.to_owned()).or_default();
        if watch.running {
            return None;
        }
        watch.running = true;
        Some(PassGuard {
            sleeper: Arc::clone(self),
            conversation_id: conversation_id.to_owned(),
        })
    }

    /// A turn began: whatever quiet-rule timer was armed is now stale.
    pub(crate) fn turn_started(&self, conversation_id: &str) {
        self.bump(conversation_id);
    }

    /// A turn landed in `conversation_id`, memory on, `agent_id` the brain it
    /// sleeps into. Ripe now → sleep now; otherwise arm the quiet rule.
    pub(crate) fn turn_completed(self: &Arc<Self>, conversation_id: String, agent_id: String) {
        // Muninn distils as it goes — there is no pass to run, and the
        // command refuses the same way.
        if !matches!(
            MemoryTarget::resolve(&self.service.companions, &agent_id),
            Ok(MemoryTarget::Organ { .. })
        ) {
            return;
        }
        let generation = self.bump(&conversation_id);
        let sleeper = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            if sleeper.fresh_turns(&conversation_id).await >= RIPE_TURNS {
                sleeper.run(&conversation_id, &agent_id).await;
                return;
            }
            tokio::time::sleep(IDLE_AFTER).await;
            if sleeper.generation(&conversation_id) != generation {
                return; // a newer turn re-armed the rule
            }
            if sleeper.fresh_turns(&conversation_id).await >= IDLE_MIN_TURNS {
                sleeper.run(&conversation_id, &agent_id).await;
            }
        });
    }

    fn bump(&self, conversation_id: &str) -> u64 {
        let Ok(mut watches) = self.watches.lock() else {
            return 0;
        };
        let watch = watches.entry(conversation_id.to_owned()).or_default();
        watch.generation += 1;
        watch.generation
    }

    fn generation(&self, conversation_id: &str) -> u64 {
        self.watches
            .lock()
            .ok()
            .and_then(|watches| watches.get(conversation_id).map(|w| w.generation))
            .unwrap_or(0)
    }

    fn backing_off(&self, conversation_id: &str) -> bool {
        self.watches
            .lock()
            .ok()
            .and_then(|watches| watches.get(conversation_id).and_then(|w| w.backoff_until))
            .is_some_and(|until| Instant::now() < until)
    }

    async fn fresh_turns(&self, conversation_id: &str) -> usize {
        let service = Arc::clone(&self.service);
        let conversation_id = conversation_id.to_owned();
        tauri::async_runtime::spawn_blocking(move || service.chats.count_unslept(&conversation_id))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(0)
    }

    async fn run(self: &Arc<Self>, conversation_id: &str, agent_id: &str) {
        if self.backing_off(conversation_id) {
            return;
        }
        let Some(guard) = self.begin(conversation_id) else {
            return; // a manual /sleep has the thread; it will claim these turns
        };
        let result = run_sleep_pass(
            Arc::clone(&self.service),
            conversation_id.to_owned(),
            agent_id.to_owned(),
            None,
        )
        .await;
        drop(guard);

        let event = match result {
            Ok(Some(SleepOutcome { created, updated, dropped, memories, scribe_note, .. })) => {
                SleptEvent::Carved {
                    conversation_id: conversation_id.to_owned(),
                    created,
                    updated,
                    dropped,
                    memories,
                    scribe_note,
                }
            }
            // Nothing fresh after all (a manual pass got there first) — nothing to say.
            Ok(None) => return,
            Err(message) => {
                eprintln!("[memory] the sleeper's pass failed on {conversation_id}: {message}");
                if let Ok(mut watches) = self.watches.lock() {
                    let watch = watches.entry(conversation_id.to_owned()).or_default();
                    watch.backoff_until = Some(Instant::now() + BACKOFF_AFTER_ERROR);
                }
                SleptEvent::Failed {
                    conversation_id: conversation_id.to_owned(),
                    message,
                }
            }
        };
        let _ = self.app.emit(SLEPT_EVENT, event);
    }
}
