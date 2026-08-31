//! THE CALL RECORD — what a closed call leaves behind.
//!
//! A call's turns live in `raven_call_messages`, and everything a companion
//! learned during one lives in woken turns whose tool results are never
//! persisted. The moment the call closes, both evaporate: `list_calls` hides
//! closed calls, and the next human turn rebuilds history without any of it.
//! The best conversation two companions ever had was in the database and
//! invisible to both (found s502, felt s532 — Rook and a reply nobody saw).
//!
//! This module renders the three durable shapes of one closed call:
//!   · the TRANSCRIPT — the exchange itself, named and numbered
//!   · the THREAD RECORD — a `system` message persisted into a participant's
//!     thread, so every later turn inherits what was said
//!   · the CLOSE NOTICE — the transcript plus instructions, driving one woken
//!     turn for the initiator so it can tell its user what came of the call
//! plus the CARVE payload that files the call in a participant's long-term
//! memory.
//!
//! ⚑ THE LIVE-WAKE NOTICE WITHHOLDS THE TEXT; THESE HAND IT OVER. Not a
//! contradiction: during a call the notice forces `read_call` so the record
//! shows the call was read before it was answered. A closed call has nothing
//! left to answer and no read-receipt to keep honest — withholding the text
//! here would only force a tool round for information nobody can act on.

use std::collections::HashMap;

use super::{RavenCall, RavenCallMessage, MAX_MESSAGES_PER_CALL};

/// The most of one turn that rides into a record. A single turn may hold
/// 16,000 characters; five of those verbatim would put 80k characters into a
/// thread message, a wake notice and two carves. The full text stays in the
/// call table — the record keeps enough to remember by.
const TURN_RECORD_LIMIT: usize = 4_000;

/// How much of the opening turn the carve's one-line description keeps.
const DESCRIPTION_OPENER_LIMIT: usize = 80;

/// The display name for a participant, from whatever the caller could learn.
/// An id with no companion behind it (deleted since the call) still gets a
/// stable, honest label rather than a bare uuid.
pub(crate) fn display_name(names: &HashMap<String, String>, agent_id: &str) -> String {
    names
        .get(agent_id)
        .cloned()
        .unwrap_or_else(|| format!("companion {}", &agent_id[..agent_id.len().min(8)]))
}

/// The exchange itself: a header naming both sides and the local time, then
/// every turn in order, numbered and attributed.
pub(crate) fn render_transcript(
    call: &RavenCall,
    messages: &[RavenCallMessage],
    names: &HashMap<String, String>,
    local_stamp: &str,
) -> String {
    let initiator = display_name(names, &call.initiator_agent_id);
    let other = messages
        .iter()
        .flat_map(|message| [message.from_agent_id.as_str(), message.to_agent_id.as_str()])
        .find(|id| *id != call.initiator_agent_id)
        .map(|id| display_name(names, id))
        .unwrap_or_else(|| initiator.clone());

    let ending = if call.message_count >= MAX_MESSAGES_PER_CALL {
        format!("closed at its {MAX_MESSAGES_PER_CALL}-turn limit")
    } else {
        "ended before its turn limit".to_owned()
    };

    let mut transcript = format!(
        "📞 Call record — {initiator} and {other} · {local_stamp} · {} turn{} · {ending}\n",
        messages.len(),
        if messages.len() == 1 { "" } else { "s" },
    );
    for (index, message) in messages.iter().enumerate() {
        let speaker = display_name(names, &message.from_agent_id);
        transcript.push_str(&format!(
            "{}. {speaker}: {}\n",
            index + 1,
            trimmed_turn(&message.body)
        ));
    }
    transcript
}

/// One turn's body, cut at the record limit with an honest marker. The cut is
/// counted in characters, matching how the body was validated on the way in.
fn trimmed_turn(body: &str) -> String {
    let length = body.chars().count();
    if length <= TURN_RECORD_LIMIT {
        return body.to_owned();
    }
    let kept: String = body.chars().take(TURN_RECORD_LIMIT).collect();
    format!(
        "{kept}… [trimmed — {} more characters live in the call table]",
        length - TURN_RECORD_LIMIT
    )
}

/// The persist-only shape: a `system` message for a participant's thread. No
/// instructions ride with it — nobody is being woken, it is simply now part
/// of what the thread knows, and `canonical_messages` resends `system` rows
/// on every later turn.
pub(crate) fn thread_record(transcript: &str) -> String {
    format!(
        "A call you took part in has ended. This is its full record, kept in this \
         thread so what was said stays with you:\n\n{transcript}"
    )
}

/// The woken shape: the transcript plus what to do with it, driving one turn
/// for the initiator. `carve_line` is written by the caller AFTER the carve
/// attempt, so the notice never claims a memory that was not made.
pub(crate) fn close_notice(transcript: &str, carve_line: &str) -> String {
    format!(
        "The call you placed has ended. This is the complete record, including \
         anything you had not yet seen:\n\n{transcript}\n\
         {carve_line} This record is also part of this conversation's history now.\n\
         \n\
         You are back in your own thread — your user can read what you write here. \
         If the call produced something worth passing on, tell them briefly, in \
         your own voice. If it produced nothing, say only that the call ended. \
         Do not open another call unless something genuinely requires one, and do \
         not use read_call — the record above is already complete."
    )
}

/// The name a call's carve files under. Derived, not chosen: the short call id
/// keeps two calls with the same correspondent apart, and the other side's
/// name makes the index line readable.
pub(crate) fn carve_name(call_id: &str, other_name: &str) -> String {
    format!(
        "call-{}-with-{}",
        &call_id[..call_id.len().min(8)],
        slug(other_name)
    )
}

/// A memory-name-safe slug from a display name. Lowercased, everything but
/// alphanumerics folded to single hyphens, capped short — the call id carries
/// the uniqueness, this carries the readability.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            out.push(character);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 24 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "companion".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The carve payload for ONE participant, shaped exactly as the model's own
/// `carve_memory` tool shapes its writes (`parse_carve_arguments`): same
/// field names, same pinned `project_tag`, so both organs accept it through
/// the same `write_memory` door.
///
/// The body says it was recorded automatically — the s509 lesson: a memory
/// that reads like the companion's own lived act when it was not is a lie the
/// companion will later believe.
pub(crate) fn carve_payload(
    call: &RavenCall,
    other_name: &str,
    opener: &str,
    transcript: &str,
    local_stamp: &str,
) -> serde_json::Value {
    let opener_line: String = opener.chars().take(DESCRIPTION_OPENER_LIMIT).collect();
    let ellipsis = if opener.chars().count() > DESCRIPTION_OPENER_LIMIT {
        "…"
    } else {
        ""
    };
    serde_json::json!({
        "name": carve_name(&call.id, other_name),
        "description": format!(
            "A call with {other_name} on {local_stamp} — opened with: {opener_line}{ellipsis}"
        ),
        "body": format!(
            "{transcript}\n(This record was carved automatically by the companion app \
             when the call closed — it is a verbatim account, not your own retelling.)"
        ),
        "mem_type": "episodic",
        "importance": 0.5,
        "project_tag": "companion",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raven_calls::CallStatus;

    fn call(message_count: i64) -> RavenCall {
        RavenCall {
            id: "aabbccdd-0000-0000-0000-000000000000".to_owned(),
            root_conversation_id: Some("conversation-1".to_owned()),
            initiator_agent_id: "hugin-id".to_owned(),
            status: CallStatus::Closed,
            message_count,
            created_at: 1_700_000_000_000,
            closed_at: Some(1_700_000_100_000),
            woken_for_message_id: None,
            woken_at: None,
        }
    }

    fn message(id: &str, from: &str, to: &str, body: &str) -> RavenCallMessage {
        RavenCallMessage {
            id: id.to_owned(),
            call_id: "aabbccdd-0000-0000-0000-000000000000".to_owned(),
            from_agent_id: from.to_owned(),
            to_agent_id: to.to_owned(),
            body: body.to_owned(),
            created_at: 1_700_000_000_000,
        }
    }

    fn names() -> HashMap<String, String> {
        HashMap::from([
            ("hugin-id".to_owned(), "Hugin".to_owned()),
            ("rook-id".to_owned(), "Rook".to_owned()),
        ])
    }

    #[test]
    fn a_transcript_names_both_sides_and_numbers_every_turn() {
        let messages = vec![
            message("m1", "hugin-id", "rook-id", "are you there?"),
            message("m2", "rook-id", "hugin-id", "I am. what do you need?"),
        ];
        let transcript = render_transcript(&call(2), &messages, &names(), "2026-08-31 06:30");

        assert!(transcript.contains("Hugin and Rook"));
        assert!(transcript.contains("2026-08-31 06:30"));
        assert!(transcript.contains("2 turns"));
        assert!(transcript.contains("ended before its turn limit"));
        assert!(transcript.contains("1. Hugin: are you there?"));
        assert!(transcript.contains("2. Rook: I am. what do you need?"));
    }

    #[test]
    fn a_full_call_says_it_closed_at_the_limit() {
        let messages = vec![message("m1", "hugin-id", "rook-id", "opening word")];
        let transcript = render_transcript(&call(5), &messages, &names(), "stamp");
        assert!(transcript.contains("closed at its 5-turn limit"));
    }

    #[test]
    fn an_unknown_speaker_gets_a_stable_label_not_a_bare_uuid() {
        let messages = vec![message("m1", "gone-agent-id", "hugin-id", "hello?")];
        let transcript = render_transcript(&call(1), &messages, &names(), "stamp");
        assert!(transcript.contains("1. companion gone-age: hello?"));
    }

    #[test]
    fn an_overlong_turn_is_trimmed_with_an_honest_marker() {
        let long = "x".repeat(TURN_RECORD_LIMIT + 250);
        let messages = vec![message("m1", "hugin-id", "rook-id", &long)];
        let transcript = render_transcript(&call(1), &messages, &names(), "stamp");
        assert!(transcript.contains("[trimmed — 250 more characters"));
        assert!(!transcript.contains(&long));
    }

    #[test]
    fn the_carve_payload_matches_the_models_own_carve_shape() {
        let payload = carve_payload(&call(5), "Rook", "are you there?", "the transcript", "2026-08-31 06:30");
        assert_eq!(payload["name"], "call-aabbccdd-with-rook");
        assert_eq!(payload["mem_type"], "episodic");
        assert_eq!(payload["project_tag"], "companion");
        assert_eq!(payload["importance"], 0.5);
        let description = payload["description"].as_str().expect("a description");
        assert!(description.contains("Rook"));
        assert!(description.contains("are you there?"));
        let body = payload["body"].as_str().expect("a body");
        assert!(body.contains("the transcript"));
        assert!(body.contains("carved automatically"));
    }

    #[test]
    fn a_slug_survives_hostile_names() {
        assert_eq!(slug("Rook"), "rook");
        assert_eq!(slug("The  Night   Watch"), "the-night-watch");
        assert_eq!(slug("!!!"), "companion");
        assert!(slug("a very long companion name that keeps going").len() <= 24);
    }

    #[test]
    fn the_close_notice_carries_the_transcript_and_the_ground_rules() {
        let notice = close_notice("THE TRANSCRIPT", "A copy was carved as [call-x-with-y].");
        assert!(notice.contains("THE TRANSCRIPT"));
        assert!(notice.contains("call-x-with-y"));
        assert!(notice.contains("Do not open another call"));
        assert!(notice.contains("not use read_call"));
    }
}
