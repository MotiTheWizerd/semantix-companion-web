//! RAVEN CALLS — an exchange between two companions, with a home.
//!
//! `agent_mail` is a mailbox: a letter addressed to an id, with no account of
//! why it was written. A CALL is the container that gives one. It is born out
//! of a specific human conversation, it holds the turns of one exchange, and
//! it carries the budget that stops that exchange running away.
//!
//! ⚑ WHY THE CONTAINER EXISTS AT ALL.
//! Two agents that can each wake the other are an unbounded loop with a token
//! meter attached: Rook answers Hugin, which wakes Rook, at three in the
//! morning, on someone's key. Scoping every exchange to a call makes the loop
//! stoppable — the limit is read off one row before a hop is allowed, rather
//! than inferred from a table that is growing while you scan it.
//!
//! THE TWO LIMITS, set by Moti and deliberately blunt until we have data:
//!   · 5 CALLS PER COMPANION PER DAY, in total — not per correspondent. A
//!     companion that makes friends does not thereby earn more budget.
//!   · 5 MESSAGES PER CALL, counting both sides together.
//!
//! "A day" means the LOCAL calendar day — the clock on the machine the
//! companion runs on, which is its owner's clock. Not UTC, and not a rolling
//! 24-hour window: a person reasoning about "five calls today" means the day
//! they are living in, and a limit nobody can predict the reset of is a limit
//! that feels broken.
//!
//! ⚑ A REPLY INHERITS THE SENDER'S CALL. It does not open one of its own.
//! Otherwise the answer arrives with a fresh budget, the chain never touches a
//! ceiling, and both limits above are decorative.

use std::{path::Path, sync::Arc};

use serde::Serialize;
use tauri::State;

use crate::app_error::AppError;

mod repository;
mod waker;

pub(crate) use repository::RavenCallRepository;
pub(crate) use waker::spawn as spawn_waker;

/// Emitted app-wide when a call changed under its own steam, so a window
/// showing one knows to look again.
pub(crate) const CALLS_CHANGED_EVENT: &str = "calls://changed";

/// Emitted the moment the waker starts a woken turn for a call — the honest
/// "a reply is in flight" signal. Every wake is followed by CALLS_CHANGED_EVENT
/// when the attempt ends, whichever way it ends, so a listener that arms on
/// this and disarms on that can never be left holding a stale indicator.
pub(crate) const CALL_WAKE_EVENT: &str = "calls://wake";

/// Calls one companion may open between local midnight and local midnight.
/// Counted across every recipient — the cap is on the companion, not the pair.
pub(crate) const MAX_CALLS_PER_DAY: i64 = 5;

/// Turns one call may hold, both sides together. Five total is roughly two
/// round trips and a closing word: enough to ask and be answered, short enough
/// that a confused pair cannot talk all night.
pub(crate) const MAX_MESSAGES_PER_CALL: i64 = 5;

/// The longest a single turn may be. Same ceiling as a letter in `agent_mail` —
/// generous for prose, low enough that a runaway agent cannot write the disk
/// full one message at a time.
const MAX_BODY_LENGTH: usize = 16_000;

/// One exchange. `root_conversation_id` is the human conversation it was born
/// out of, and `None` means unrooted — a scheduled wake, or a companion writing
/// with nobody watching. Unrooted calls still spend from the daily allowance,
/// because that allowance belongs to the companion rather than to any thread.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RavenCall {
    pub(crate) id: String,
    pub(crate) root_conversation_id: Option<String>,
    pub(crate) initiator_agent_id: String,
    pub(crate) status: CallStatus,
    /// Kept on the row rather than counted on demand. This is the number the
    /// cap check reads on every hop, and the message table it would otherwise
    /// scan is the one that grows at machine speed.
    pub(crate) message_count: i64,
    pub(crate) created_at: i64,
    pub(crate) closed_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CallStatus {
    Open,
    Closed,
}

impl CallStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "closed" => Some(Self::Closed),
            _ => None,
        }
    }
}

/// One turn inside a call.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RavenCallMessage {
    pub(crate) id: String,
    pub(crate) call_id: String,
    pub(crate) from_agent_id: String,
    pub(crate) to_agent_id: String,
    pub(crate) body: String,
    pub(crate) created_at: i64,
}

/// A call waiting on someone, and the turn it is waiting because of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingWake {
    pub(crate) call_id: String,
    /// The newest turn in the call. Doubles as the wake guard's key — a
    /// companion is woken at most once per thing said to it.
    pub(crate) message_id: String,
    /// Who owes a reply: the recipient of that turn.
    pub(crate) agent_id: String,
}

/// A call and everything said in it — what a person needs to see one, in a
/// single trip rather than a call list plus a fetch per row.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CallThread {
    pub(crate) call: RavenCall,
    pub(crate) messages: Vec<RavenCallMessage>,
}

/// Turns loaded per call for the UI. A call cannot exceed
/// `MAX_MESSAGES_PER_CALL`, so this only bites if that limit is raised — but
/// the query is bounded on principle, because this is the table that grows.
const THREAD_MESSAGE_LIMIT: i64 = 50;

pub(crate) struct RavenCallState {
    repository: Arc<RavenCallRepository>,
}

impl RavenCallState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            repository: Arc::new(RavenCallRepository::open(database_path)?),
        })
    }

    /// A second handle on the store for the waker, which outlives any command.
    pub(crate) fn repository(&self) -> Arc<RavenCallRepository> {
        Arc::clone(&self.repository)
    }
}

/// Every call born out of one conversation, newest first, with its turns.
///
/// Read-only and conversation-scoped. There is deliberately no command that
/// returns calls across conversations or by agent: the human surface answers
/// "what was said on my behalf HERE", and a wider door would have to justify
/// itself separately.
#[tauri::command]
pub(crate) async fn list_conversation_calls(
    state: State<'_, RavenCallState>,
    conversation_id: String,
) -> Result<Vec<CallThread>, String> {
    let repository = Arc::clone(&state.repository);
    tauri::async_runtime::spawn_blocking(move || {
        let calls = repository.calls_for_conversation(&conversation_id)?;
        calls
            .into_iter()
            .map(|call| {
                let messages = repository.messages(&call.id, THREAD_MESSAGE_LIMIT)?;
                Ok(CallThread { call, messages })
            })
            .collect::<Result<Vec<_>, AppError>>()
    })
    .await
    .map_err(|error| format!("Call task failed: {error}"))?
    .map_err(String::from)
}
