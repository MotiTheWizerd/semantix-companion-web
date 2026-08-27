//! Provider-blind projection of streamed tool arguments into live call speech.
//!
//! Providers normalize their native events into `ToolCallDelta`; this module
//! knows only the canonical tool name and JSON arguments. Tool execution still
//! waits for the final validated `ToolCall` — these drafts are UI-only.

use std::collections::HashMap;

use serde::Deserialize;

use crate::{
    inference::{ToolCall, ToolCallDelta},
    tools::SEND_IN_CALL,
};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct CallSpeechChunk {
    pub(super) stream_id: String,
    pub(super) call_id: String,
    pub(super) delta: String,
}

#[derive(Default)]
struct ToolDraft {
    name: String,
    arguments: String,
    emitted_body: String,
}

#[derive(Default)]
pub(super) struct CallSpeechProjector {
    drafts: HashMap<String, ToolDraft>,
}

impl CallSpeechProjector {
    pub(super) fn absorb(&mut self, fragment: &ToolCallDelta) -> Option<CallSpeechChunk> {
        let draft = self.drafts.entry(fragment.id.clone()).or_default();
        if !fragment.name.is_empty() {
            draft.name.clone_from(&fragment.name);
        }
        draft.arguments.push_str(&fragment.arguments_delta);
        if draft.name != SEND_IN_CALL {
            return None;
        }

        // The destination must be a complete string before anything appears
        // in its card. The body may be incomplete: that is the part we stream.
        let (call_id, call_id_complete) = partial_json_string_field(&draft.arguments, "call_id")?;
        if !call_id_complete || call_id.is_empty() {
            return None;
        }
        let (body, _) = partial_json_string_field(&draft.arguments, "body")?;
        let delta = body.strip_prefix(&draft.emitted_body)?.to_owned();
        if delta.is_empty() {
            return None;
        }
        draft.emitted_body = body;
        Some(CallSpeechChunk {
            stream_id: fragment.id.clone(),
            call_id,
            delta,
        })
    }
}

#[derive(Deserialize)]
struct SendInCallArguments {
    call_id: String,
    body: String,
}

/// The completed call is authoritative. This is used only after execution to
/// tell the UI whether its transient draft can be reconciled with SQLite.
pub(super) fn completed_call_speech(call: &ToolCall) -> Option<(String, String)> {
    if call.name != SEND_IN_CALL {
        return None;
    }
    let arguments: SendInCallArguments = serde_json::from_str(&call.arguments).ok()?;
    Some((arguments.call_id, arguments.body))
}

/// Decode one JSON string while its containing object is still arriving.
/// Complete escape sequences are decoded; an incomplete trailing escape is
/// held until its next fragment rather than leaking raw JSON into the UI.
fn partial_json_string_field(input: &str, field: &str) -> Option<(String, bool)> {
    let needle = format!(r#""{field}""#);
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut field_at = None;
    for (offset, character) in input.char_indices() {
        if in_string {
            if character == '"' && !escaped {
                in_string = false;
            }
            if character == '\\' {
                escaped = !escaped;
            } else {
                escaped = false;
            }
            continue;
        }
        match character {
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            '"' if depth == 1 && input[offset..].starts_with(&needle) => {
                field_at = Some(offset);
                break;
            }
            '"' => in_string = true,
            _ => {}
        }
    }
    let after_name = input.get(field_at? + needle.len()..)?;
    let after_colon = after_name.get(after_name.find(':')? + 1..)?.trim_start();
    let raw = after_colon.strip_prefix('"')?;

    let mut escaped = false;
    let mut closing = None;
    for (offset, character) in raw.char_indices() {
        if character == '"' && !escaped {
            closing = Some(offset);
            break;
        }
        if character == '\\' {
            escaped = !escaped;
        } else {
            escaped = false;
        }
    }

    let candidate = &raw[..closing.unwrap_or(raw.len())];
    let mut end = candidate.len();
    loop {
        let encoded = format!("\"{}\"", &candidate[..end]);
        if let Ok(decoded) = serde_json::from_str::<String>(&encoded) {
            return Some((decoded, closing.is_some()));
        }
        end = candidate[..end].char_indices().next_back()?.0;
    }
}

#[cfg(test)]
mod tests {
    use super::{completed_call_speech, partial_json_string_field, CallSpeechProjector};
    use crate::{
        inference::{ToolCall, ToolCallDelta},
        tools::SEND_IN_CALL,
    };

    fn fragment(id: &str, arguments_delta: &str) -> ToolCallDelta {
        ToolCallDelta {
            id: id.to_owned(),
            name: SEND_IN_CALL.to_owned(),
            arguments_delta: arguments_delta.to_owned(),
        }
    }

    #[test]
    fn projects_only_new_decoded_body_text() {
        let mut projector = CallSpeechProjector::default();
        assert_eq!(
            projector.absorb(&fragment("tool-1", r#"{"call_id":"call-9","body":"Hel"#)),
            Some(super::CallSpeechChunk {
                stream_id: "tool-1".to_owned(),
                call_id: "call-9".to_owned(),
                delta: "Hel".to_owned(),
            })
        );
        assert_eq!(
            projector.absorb(&fragment("tool-1", r#"lo\nworld"}"#)),
            Some(super::CallSpeechChunk {
                stream_id: "tool-1".to_owned(),
                call_id: "call-9".to_owned(),
                delta: "lo\nworld".to_owned(),
            })
        );
    }

    #[test]
    fn waits_for_a_complete_call_id_and_incomplete_escape() {
        let mut projector = CallSpeechProjector::default();
        assert!(projector
            .absorb(&fragment("tool-2", r#"{"call_id":"call"#))
            .is_none());
        let chunk = projector
            .absorb(&fragment("tool-2", "-2\",\"body\":\"say \\"))
            .expect("the closed call id should release the safe body prefix");
        assert_eq!(chunk.call_id, "call-2");
        assert_eq!(chunk.delta, "say ");
        let chunk = projector
            .absorb(&fragment("tool-2", r#"nhello"}"#))
            .expect("the completed escape should stream once it is valid");
        assert_eq!(chunk.delta, "\nhello");
    }

    #[test]
    fn ignores_unrelated_tools() {
        let mut projector = CallSpeechProjector::default();
        let mut delta = fragment("tool-3", r#"{"call_id":"call-3","body":"secret"}"#);
        delta.name = "carve_memory".to_owned();
        assert!(projector.absorb(&delta).is_none());
    }

    #[test]
    fn completed_send_in_call_arguments_are_recovered() {
        let speech = completed_call_speech(&ToolCall {
            id: "provider-call".to_owned(),
            name: SEND_IN_CALL.to_owned(),
            arguments: r#"{"call_id":"call-4","body":"All done."}"#.to_owned(),
        });
        assert_eq!(speech, Some(("call-4".to_owned(), "All done.".to_owned())));
    }

    #[test]
    fn partial_string_decoder_handles_quotes_and_unicode_boundaries() {
        assert_eq!(
            partial_json_string_field(r#"{"body":"hi \"you\" \u263A"#, "body"),
            Some(("hi \"you\" ☺".to_owned(), false))
        );
        assert_eq!(
            partial_json_string_field(r#"{"body":"hi \u26"#, "body"),
            Some(("hi ".to_owned(), false))
        );
    }
}
