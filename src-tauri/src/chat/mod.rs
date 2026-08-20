mod repository;

use std::{path::Path, sync::Arc};

use repository::ChatRepository;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};
use uuid::Uuid;

use crate::{app_error::AppError, credentials::unix_timestamp_ms};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Conversation {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) selected_model_id: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) archived_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Message {
    pub(crate) id: String,
    pub(crate) conversation_id: String,
    pub(crate) sequence: i64,
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) content: String,
    pub(crate) provider_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) completed_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConversationThread {
    pub(crate) conversation: Conversation,
    pub(crate) messages: Vec<Message>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcceptedMessage {
    pub(crate) conversation: Conversation,
    pub(crate) message: Message,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubmitMessageInput {
    conversation_id: Option<String>,
    content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[allow(dead_code)] // Provider adapters will drive the full stream protocol in the next slice.
pub(crate) enum ChatEvent {
    Accepted {
        conversation: Conversation,
        message: Message,
    },
    AssistantStarted {
        message: Message,
    },
    AssistantDelta {
        #[serde(rename = "conversationId")]
        conversation_id: String,
        #[serde(rename = "messageId")]
        message_id: String,
        delta: String,
    },
    AssistantCompleted {
        message: Message,
    },
    Failed {
        #[serde(rename = "conversationId")]
        conversation_id: String,
        #[serde(rename = "messageId")]
        message_id: Option<String>,
        message: String,
    },
}

pub(crate) struct ChatState {
    service: Arc<ChatService>,
}

impl ChatState {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            service: Arc::new(ChatService {
                repository: ChatRepository::open(database_path)?,
            }),
        })
    }
}

struct ChatService {
    repository: ChatRepository,
}

impl ChatService {
    fn list_conversations(&self) -> Result<Vec<Conversation>, AppError> {
        self.repository.list_conversations()
    }

    fn get_thread(&self, conversation_id: &str) -> Result<ConversationThread, AppError> {
        self.repository
            .get_thread(conversation_id)?
            .ok_or_else(|| AppError::validation("That conversation no longer exists."))
    }

    fn submit(&self, input: SubmitMessageInput) -> Result<AcceptedMessage, AppError> {
        let content = input.content.trim();
        if content.is_empty() {
            return Err(AppError::validation("Write a message before sending."));
        }
        if content.chars().count() > 100_000 {
            return Err(AppError::validation(
                "Messages must be 100,000 characters or fewer.",
            ));
        }

        let conversation_id = input
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let timestamp = unix_timestamp_ms()?;
        self.repository.commit_user_message(
            conversation_id,
            content,
            &conversation_title(content),
            timestamp,
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
        )
    }
}

#[tauri::command]
pub(crate) async fn list_conversations(
    state: State<'_, ChatState>,
) -> Result<Vec<Conversation>, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.list_conversations())
        .await
        .map_err(|error| format!("Conversation task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn get_conversation_thread(
    state: State<'_, ChatState>,
    conversation_id: String,
) -> Result<ConversationThread, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.get_thread(&conversation_id))
        .await
        .map_err(|error| format!("Conversation task failed: {error}"))?
        .map_err(String::from)
}

#[tauri::command]
pub(crate) async fn submit_message(
    state: State<'_, ChatState>,
    input: SubmitMessageInput,
    on_event: Channel<ChatEvent>,
) -> Result<AcceptedMessage, String> {
    let service = Arc::clone(&state.service);
    let accepted = tauri::async_runtime::spawn_blocking(move || service.submit(input))
        .await
        .map_err(|error| format!("Message task failed: {error}"))?
        .map_err(String::from)?;

    let _ = on_event.send(ChatEvent::Accepted {
        conversation: accepted.conversation.clone(),
        message: accepted.message.clone(),
    });
    Ok(accepted)
}

fn conversation_title(content: &str) -> String {
    let title = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect::<String>();
    if title.is_empty() {
        "New conversation".to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::{
        conversation_title, repository::ChatRepository, ChatEvent, ChatService, SubmitMessageInput,
    };
    use crate::database;

    #[test]
    fn conversation_titles_are_compact_and_single_line() {
        assert_eq!(
            conversation_title("  A quiet\n beginning   for Companion  "),
            "A quiet beginning for Companion"
        );
        assert_eq!(conversation_title(&"x".repeat(80)).chars().count(), 60);
    }

    #[test]
    fn stream_events_use_the_camel_case_ipc_contract() {
        let event = serde_json::to_value(ChatEvent::AssistantDelta {
            conversation_id: "conversation-123".to_owned(),
            message_id: "message-123".to_owned(),
            delta: "hello".to_owned(),
        })
        .expect("chat event should serialize");

        assert_eq!(event["kind"], "assistantDelta");
        assert_eq!(event["conversationId"], "conversation-123");
        assert_eq!(event["messageId"], "message-123");
        assert!(event.get("conversation_id").is_none());
        assert!(event.get("message_id").is_none());
    }

    #[test]
    fn submitted_messages_persist_and_reload_as_a_thread() {
        let database_path = std::env::temp_dir().join(format!(
            "semantix-companion-chat-test-{}.db",
            Uuid::new_v4()
        ));
        database::initialise(&database_path).expect("test database should initialise");

        {
            let service = ChatService {
                repository: ChatRepository::open(&database_path)
                    .expect("chat repository should open"),
            };
            let accepted = service
                .submit(SubmitMessageInput {
                    conversation_id: None,
                    content: "Remember that my favorite ship is the Long Serpent.".to_owned(),
                })
                .expect("message should persist");

            let conversations = service
                .list_conversations()
                .expect("conversations should reload");
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].id, accepted.conversation.id);

            let thread = service
                .get_thread(&accepted.conversation.id)
                .expect("thread should reload");
            assert_eq!(thread.messages.len(), 1);
            assert_eq!(thread.messages[0].content, accepted.message.content);
            assert_eq!(thread.messages[0].role, "user");
            assert_eq!(thread.messages[0].status, "completed");
        }

        for path in [
            database_path.clone(),
            database_path.with_extension("db-wal"),
            database_path.with_extension("db-shm"),
        ] {
            let _ = fs::remove_file(path);
        }
    }
}
