//! AGENT MAIL — one companion writing to another.
//!
//! Distinct from `chat`, which is a person talking to a companion inside a
//! conversation. These messages have no thread and no turn order: an agent
//! addresses another agent by id, and the letter waits until it is read.
//!
//! THE POSTURE, set by Moti for a product with real users:
//!   · ANYONE MAY SEND. There is no allow-list, no acceptance step. A companion
//!     can write to any agent whose id it knows.
//!   · NOBODY READS ANOTHER'S MAIL. The only reads exposed are "addressed to
//!     me" and "sent by me". There is deliberately no query that returns a
//!     third party's correspondence, so the privacy is a property of the
//!     surface rather than a rule someone has to remember.
//!
//! That asymmetry is the whole security model, and it is the same one email
//! has. It leaves an unsolicited-mail vector open on purpose — the answer to
//! that is blocking and reporting later, not a smaller inbox now.

// The store ships before its callers: migration, repository and tests are
// complete, and the IPC commands + inbox UI are the next slice. DELETE THIS
// ALLOW when they land — it exists to keep one deliberate gap quiet, not to
// make dead code permanently invisible in this module.
#![allow(dead_code)]

use serde::Serialize;

use crate::app_error::AppError;

mod repository;

// The store is complete and tested; nothing calls it yet. The IPC commands and
// the inbox UI are the next slice, and this re-export is what they will reach
// for — kept here so the seam is visible rather than discovered later.
#[allow(unused_imports)]
pub(crate) use repository::AgentMailRepository;

/// A single letter. `read_at: None` is the unread state and the only flag the
/// inbox badge needs.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentMessage {
    pub(crate) id: String,
    pub(crate) from_agent_id: String,
    pub(crate) to_agent_id: String,
    /// The accounts each side belongs to. Both `None` while everything lives on
    /// one machine; split by side because a cross-user letter belongs to two
    /// accounts and one column could not say which end was which.
    pub(crate) from_user_id: Option<String>,
    pub(crate) to_user_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) body: String,
    pub(crate) created_at: i64,
    pub(crate) read_at: Option<i64>,
}

/// What a caller must supply to send. Separate from `AgentMessage` because the
/// id and timestamp are the repository's to mint — a caller that could choose
/// its own `created_at` could forge the order of a correspondence.
#[derive(Debug)]
pub(crate) struct SendAgentMessage<'a> {
    pub(crate) from_agent_id: &'a str,
    pub(crate) to_agent_id: &'a str,
    pub(crate) from_user_id: Option<&'a str>,
    pub(crate) to_user_id: Option<&'a str>,
    pub(crate) project_id: Option<&'a str>,
    pub(crate) body: &'a str,
}

/// The longest a single letter may be. Generous for prose, and low enough that
/// a runaway agent cannot write the disk full one message at a time.
const MAX_BODY_LENGTH: usize = 16_000;

impl SendAgentMessage<'_> {
    /// Validated here rather than at the database, so a caller gets a sentence
    /// it can show a user instead of a constraint violation.
    fn validate(&self) -> Result<String, AppError> {
        for (label, value) in [
            ("sender", self.from_agent_id),
            ("recipient", self.to_agent_id),
        ] {
            if value.trim().is_empty() {
                return Err(AppError::validation(format!(
                    "a message needs a {label} — an agent id, not an empty string"
                )));
            }
        }

        let body = self.body.trim();
        if body.is_empty() {
            return Err(AppError::validation("a message needs something in it"));
        }
        if body.chars().count() > MAX_BODY_LENGTH {
            return Err(AppError::validation(format!(
                "a message may be at most {MAX_BODY_LENGTH} characters"
            )));
        }

        Ok(body.to_owned())
    }
}
