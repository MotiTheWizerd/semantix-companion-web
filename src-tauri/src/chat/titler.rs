//! The titler — a conversation earns its name.
//!
//! A new thread is named after its first message, cut at sixty characters.
//! That is a placeholder wearing a title's clothes: "hey, quick question about
//! the thing we discussed yesterday" says nothing about what followed. Moti's
//! ask (s570): once the thread is two or three messages in, hand its opening
//! to the background model and let it name the thing for what it is.
//!
//! The background model is the scribe — the same OpenAI-compatible model the
//! sleeper borrows to distil memory, resolved the same way. It runs after the
//! turn has landed, off the composer's path, and reports through an app-wide
//! event so the tab and the sidebar rename themselves without a reload.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::{
    app_error::AppError,
    inference::{
        InferenceDelta, InferenceExecution, InferenceMessage, InferenceRequest, ModelTarget,
        ProviderCredential, Role,
    },
    preferences::{ModelPreference, ResolvedVoice},
    streaming::{StreamError, StreamEvent, StreamSink},
};

use super::{conversation_title, ChatService, Conversation, ConversationThread, Message};

/// App-wide: a conversation was given its real name.
pub(crate) const TITLED_EVENT: &str = "chat://titled";

/// The thread is ready for a name once this many answers have landed — by
/// then the person has said what they came for and the companion has shown
/// what the thread is going to be about.
const ANSWERS_BEFORE_TITLING: usize = 2;
/// How much of the opening the namer reads. A title comes from how a
/// conversation starts; the rest of a long thread would only dilute it.
const OPENING_TURNS: usize = 6;
/// Per-turn ceiling on what rides to the namer, in characters.
const TURN_EXCERPT_CHARS: usize = 600;
/// The sidebar's width, in characters — the same cap the placeholder wears.
const TITLE_MAX_CHARS: usize = 60;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TitledEvent {
    pub(crate) conversation_id: String,
    pub(crate) title: String,
}

/// After a human-lane turn lands: name the thread if it is due one. Never
/// blocks the turn's return — the whole pass is spawned, and a failure is a
/// log line, not an error the composer sees. The placeholder title is a
/// working title, and a working title is never wrong enough to interrupt.
pub(crate) fn title_after_turn(service: Arc<ChatService>, app: AppHandle, conversation_id: String) {
    tauri::async_runtime::spawn(async move {
        match run(service, &conversation_id).await {
            Ok(Some(conversation)) => {
                let _ = app.emit(
                    TITLED_EVENT,
                    TitledEvent {
                        conversation_id: conversation.id,
                        title: conversation.title,
                    },
                );
            }
            Ok(None) => {}
            Err(error) => eprintln!("[chat] the titler could not name {conversation_id}: {error}"),
        }
    });
}

async fn run(service: Arc<ChatService>, conversation_id: &str) -> Result<Option<Conversation>, AppError> {
    // Read + decide + resolve the model inside spawn_blocking: rusqlite and
    // the keyring are sync.
    let prepared = {
        let service = Arc::clone(&service);
        let conversation_id = conversation_id.to_owned();
        tauri::async_runtime::spawn_blocking(move || prepare(&service, &conversation_id))
            .await
            .map_err(|error| AppError::internal(format!("the titler's read failed: {error}")))??
    };
    let Some(Prepared { execution }) = prepared else {
        return Ok(None);
    };

    let sink = CollectingSink::default();
    service
        .streaming
        .stream(&execution, &sink)
        .await
        .map_err(|error| AppError::internal(error.to_string()))?;
    let Some(title) = clean_title(&sink.text()) else {
        return Err(AppError::internal("the model answered with nothing usable as a title"));
    };

    let conversation_id = conversation_id.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        service.repository.rename_conversation(&conversation_id, &title)
    })
    .await
    .map_err(|error| AppError::internal(format!("the titler's write failed: {error}")))?
    .map(Some)
}

struct Prepared {
    execution: InferenceExecution,
}

/// `Ok(None)` = not due, or no model can do it. Both are quiet outcomes.
fn prepare(service: &ChatService, conversation_id: &str) -> Result<Option<Prepared>, AppError> {
    let Some(thread) = service.repository.get_thread(conversation_id)? else {
        return Ok(None);
    };
    if !wants_title(&thread) {
        return Ok(None);
    }
    let Some((target, credential)) = namer_model(service, thread.conversation.companion_id.as_deref())?
    else {
        return Ok(None);
    };
    Ok(Some(Prepared {
        execution: InferenceExecution {
            request: InferenceRequest {
                id: Uuid::new_v4().to_string(),
                target,
                messages: vec![
                    InferenceMessage::text(Role::System, NAMER_PROMPT),
                    InferenceMessage::text(Role::User, opening_excerpt(&thread.messages)),
                ],
                tools: Vec::new(),
                session_id: None,
            },
            credential,
            tool_runner: None,
        },
    }))
}

/// Due when enough answers have landed AND the thread still wears the name
/// its first message gave it. Stateless on purpose: a model title, once
/// written, differs from the placeholder and ends the matter; a failed pass
/// leaves the placeholder standing and the next turn tries again; a thread
/// from before the titler existed gets its name on its next turn.
pub(super) fn wants_title(thread: &ConversationThread) -> bool {
    let answers = thread
        .messages
        .iter()
        .filter(|m| m.role == "assistant" && m.status == "completed" && !m.content.trim().is_empty())
        .count();
    if answers < ANSWERS_BEFORE_TITLING {
        return false;
    }
    let Some(first) = thread.messages.iter().find(|m| m.role == "user") else {
        return false;
    };
    thread.conversation.title == conversation_title(&first.content)
}

/// WHICH MODEL names the thread — the scribe's resolution, the same one
/// /sleep uses: the companion's own configured model; a Claude Code
/// companion borrows the user's default or the most recently touched
/// configured model; the test stream names nothing. Kept in step with
/// `MemoryService::resolve_scribe` so the two background hands are one.
fn namer_model(
    service: &ChatService,
    companion_id: Option<&str>,
) -> Result<Option<(ModelTarget, ProviderCredential)>, AppError> {
    let companion = service.companions.resolve(companion_id)?;
    let configured_model_id = match service.preferences.resolve_voice(&companion.model_preference)? {
        ResolvedVoice::Configured(model_id) => model_id,
        ResolvedVoice::ClaudeCode(_) => {
            match service.preferences.resolve_voice(&ModelPreference::Inherit)? {
                ResolvedVoice::Configured(model_id) => model_id,
                _ => match service.model_resolver.list()?.into_iter().next() {
                    Some(model) => model.id,
                    None => return Ok(None),
                },
            }
        }
        ResolvedVoice::TestStream => return Ok(None),
    };
    let model = service.model_resolver.resolve(&configured_model_id)?;
    Ok(Some((
        ModelTarget {
            provider_id: model.provider_id,
            model_id: model.model_id,
        },
        ProviderCredential::ApiKey(model.api_key),
    )))
}

const NAMER_PROMPT: &str = "You name conversations for a sidebar. You will be shown the opening of a \
conversation between a person and their companion. Reply with ONLY the title: two to six words, \
specific to what the conversation is actually about, in the language the person wrote in. No \
quotes, no trailing period, no preamble, no explanation.";

/// The first turns of the thread, each cut to an excerpt, as the namer reads
/// them. Attachments are not described — a title comes from the words.
fn opening_excerpt(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant") && !m.content.trim().is_empty())
        .take(OPENING_TURNS)
        .map(|m| {
            let who = if m.role == "user" { "Person" } else { "Companion" };
            let text = m.content.trim();
            let excerpt: String = text.chars().take(TURN_EXCERPT_CHARS).collect();
            let ellipsis = if excerpt.chars().count() < text.chars().count() { "…" } else { "" };
            format!("{who}: {excerpt}{ellipsis}")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// What the model said, made into a title: first non-empty line, a leading
/// "Title:" and wrapping quotes stripped, whitespace collapsed, trailing
/// period dropped, capped at the sidebar's width. `None` when nothing is left.
fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line
        .strip_prefix("Title:")
        .or_else(|| line.strip_prefix("title:"))
        .or_else(|| line.strip_prefix("TITLE:"))
        .unwrap_or(line)
        .trim();
    let line = line
        .trim_matches(|c| matches!(c, '"' | '\'' | '“' | '”' | '‘' | '’' | '*' | '#'))
        .trim()
        .trim_end_matches('.')
        .trim();
    let title: String = line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();
    let title = title.trim().to_owned();
    (!title.is_empty()).then_some(title)
}

/// Gathers the answer's text. Reasoning, usage and finish deltas pass by;
/// a namer that calls a tool is not a namer, and there are none to call.
#[derive(Default)]
struct CollectingSink {
    text: Mutex<String>,
}

impl CollectingSink {
    fn text(&self) -> String {
        self.text.lock().map(|text| text.clone()).unwrap_or_default()
    }
}

impl StreamSink<InferenceDelta> for CollectingSink {
    fn emit(&self, event: StreamEvent<InferenceDelta>) -> Result<(), StreamError> {
        if let StreamEvent::Delta {
            payload: InferenceDelta::Text { text },
            ..
        } = event
        {
            self.text
                .lock()
                .map_err(|_| StreamError::new("the titler's buffer was poisoned"))?
                .push_str(&text);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, status: &str, content: &str) -> Message {
        Message {
            id: Uuid::new_v4().to_string(),
            conversation_id: "conversation-1".to_owned(),
            sequence: 0,
            role: role.to_owned(),
            status: status.to_owned(),
            content: content.to_owned(),
            provider_id: None,
            model_id: None,
            companion_id: None,
            error_message: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
            slept_at: None,
            attachments: Vec::new(),
        }
    }

    fn thread(title: &str, messages: Vec<Message>) -> ConversationThread {
        ConversationThread {
            conversation: Conversation {
                id: "conversation-1".to_owned(),
                title: title.to_owned(),
                companion_id: None,
                created_at: 0,
                updated_at: 0,
                archived_at: None,
            },
            messages,
        }
    }

    #[test]
    fn a_thread_is_due_once_the_second_answer_lands() {
        let first = "hey can you help me plan the trip";
        let placeholder = conversation_title(first);
        let one_answer = thread(
            &placeholder,
            vec![message("user", "completed", first), message("assistant", "completed", "Sure.")],
        );
        assert!(!wants_title(&one_answer), "one answer is too early");

        let two_answers = thread(
            &placeholder,
            vec![
                message("user", "completed", first),
                message("assistant", "completed", "Sure."),
                message("user", "completed", "Lisbon, four days, in October."),
                message("assistant", "completed", "Then start with Alfama."),
            ],
        );
        assert!(wants_title(&two_answers));
    }

    #[test]
    fn a_thread_already_named_by_the_model_is_left_alone() {
        let first = "hey can you help me plan the trip";
        let named = thread(
            "Four days in Lisbon",
            vec![
                message("user", "completed", first),
                message("assistant", "completed", "Sure."),
                message("user", "completed", "Lisbon, four days."),
                message("assistant", "completed", "Alfama first."),
            ],
        );
        assert!(!wants_title(&named));
    }

    #[test]
    fn a_streaming_or_empty_answer_does_not_count() {
        let first = "quick one";
        let placeholder = conversation_title(first);
        let not_yet = thread(
            &placeholder,
            vec![
                message("user", "completed", first),
                message("assistant", "completed", "Go ahead."),
                message("user", "completed", "What is the capital of Peru?"),
                message("assistant", "streaming", "Li"),
            ],
        );
        assert!(!wants_title(&not_yet));
        let failed = thread(
            &placeholder,
            vec![
                message("user", "completed", first),
                message("assistant", "completed", "Go ahead."),
                message("user", "completed", "What is the capital of Peru?"),
                message("assistant", "failed", ""),
            ],
        );
        assert!(!wants_title(&failed));
    }

    #[test]
    fn the_answer_is_cleaned_into_a_title() {
        assert_eq!(clean_title("Four days in Lisbon"), Some("Four days in Lisbon".to_owned()));
        assert_eq!(clean_title("\"Four days in Lisbon.\"\n"), Some("Four days in Lisbon".to_owned()));
        assert_eq!(clean_title("Title: Four   days\tin Lisbon"), Some("Four days in Lisbon".to_owned()));
        assert_eq!(clean_title("\n\n  **Lisbon trip**  \nmore text"), Some("Lisbon trip".to_owned()));
        assert_eq!(clean_title("   \n\n"), None);
        assert_eq!(clean_title(&"x".repeat(80)).map(|t| t.chars().count()), Some(60));
    }

    #[test]
    fn the_excerpt_reads_the_opening_only() {
        let messages: Vec<Message> = (0..10)
            .map(|i| {
                message(
                    if i % 2 == 0 { "user" } else { "assistant" },
                    "completed",
                    &format!("turn {i}"),
                )
            })
            .collect();
        let excerpt = opening_excerpt(&messages);
        assert!(excerpt.starts_with("Person: turn 0"));
        assert!(excerpt.contains("Companion: turn 5"));
        assert!(!excerpt.contains("turn 6"));
    }
}
