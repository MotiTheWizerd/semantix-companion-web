mod repository;

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use repository::ChatRepository;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State};
use uuid::Uuid;

use crate::{
    app_error::AppError,
    credentials::unix_timestamp_ms,
    streaming::{
        StreamError, StreamingService, TestTextSource, TextStreamEvent, TextStreamRequest,
        TextStreamSink,
    },
};

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
        sequence: u64,
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
        let repository = ChatRepository::open(database_path)?;
        repository.fail_interrupted_streams(unix_timestamp_ms()?)?;
        Ok(Self {
            service: Arc::new(ChatService::new(
                repository,
                StreamingService::new(Arc::new(TestTextSource::default())),
            )),
        })
    }
}

struct ChatService {
    repository: ChatRepository,
    streaming: StreamingService,
}

impl ChatService {
    fn new(repository: ChatRepository, streaming: StreamingService) -> Self {
        Self {
            repository,
            streaming,
        }
    }

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

    fn begin_assistant(&self, conversation_id: &str) -> Result<Message, AppError> {
        self.repository.begin_assistant_message(
            conversation_id,
            &Uuid::new_v4().to_string(),
            unix_timestamp_ms()?,
        )
    }

    fn complete_assistant(&self, message_id: &str, content: &str) -> Result<Message, AppError> {
        self.repository
            .complete_assistant_message(message_id, content, unix_timestamp_ms()?)
    }

    fn fail_assistant(&self, message_id: &str, message: &str) -> Result<Message, AppError> {
        self.repository
            .fail_assistant_message(message_id, message, unix_timestamp_ms()?)
    }
}

struct ChatStreamAdapter {
    assistant: Message,
    conversation_id: String,
    message_id: String,
    on_event: Channel<ChatEvent>,
    content: Mutex<String>,
}

impl ChatStreamAdapter {
    fn new(assistant: Message, on_event: Channel<ChatEvent>) -> Self {
        Self {
            conversation_id: assistant.conversation_id.clone(),
            message_id: assistant.id.clone(),
            assistant,
            on_event,
            content: Mutex::new(String::new()),
        }
    }

    fn content(&self) -> Result<String, StreamError> {
        self.content
            .lock()
            .map(|content| content.clone())
            .map_err(|_| StreamError::new("the chat stream buffer was poisoned"))
    }
}

impl TextStreamSink for ChatStreamAdapter {
    fn emit(&self, event: TextStreamEvent) -> Result<(), StreamError> {
        match event {
            TextStreamEvent::Started => {
                let _ = self.on_event.send(ChatEvent::AssistantStarted {
                    message: self.assistant.clone(),
                });
            }
            TextStreamEvent::Delta { sequence, text } => {
                self.content
                    .lock()
                    .map_err(|_| StreamError::new("the chat stream buffer was poisoned"))?
                    .push_str(&text);
                let _ = self.on_event.send(ChatEvent::AssistantDelta {
                    conversation_id: self.conversation_id.clone(),
                    message_id: self.message_id.clone(),
                    sequence,
                    delta: text,
                });
            }
            TextStreamEvent::Completed | TextStreamEvent::Failed { .. } => {}
        }
        Ok(())
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
    let submit_service = Arc::clone(&service);
    let accepted = tauri::async_runtime::spawn_blocking(move || submit_service.submit(input))
        .await
        .map_err(|error| format!("Message task failed: {error}"))?
        .map_err(String::from)?;

    let _ = on_event.send(ChatEvent::Accepted {
        conversation: accepted.conversation.clone(),
        message: accepted.message.clone(),
    });

    let conversation_id = accepted.conversation.id.clone();
    let assistant_service = Arc::clone(&service);
    let assistant_conversation_id = conversation_id.clone();
    let assistant = match tauri::async_runtime::spawn_blocking(move || {
        assistant_service.begin_assistant(&assistant_conversation_id)
    })
    .await
    .map_err(|error| format!("Assistant task failed: {error}"))?
    {
        Ok(message) => message,
        Err(error) => {
            let message = error.to_string();
            let _ = on_event.send(ChatEvent::Failed {
                conversation_id,
                message_id: None,
                message: message.clone(),
            });
            return Err(message);
        }
    };

    let adapter = ChatStreamAdapter::new(assistant.clone(), on_event.clone());
    let request = TextStreamRequest {
        input: accepted.message.content.clone(),
    };

    if let Err(error) = service.streaming.stream(&request, &adapter).await {
        let message = error.to_string();
        return Err(fail_stream(
            Arc::clone(&service),
            &on_event,
            &assistant.conversation_id,
            &assistant.id,
            message,
        )
        .await);
    }

    let content = match adapter.content() {
        Ok(content) => content,
        Err(error) => {
            return Err(fail_stream(
                Arc::clone(&service),
                &on_event,
                &assistant.conversation_id,
                &assistant.id,
                error.to_string(),
            )
            .await);
        }
    };
    let completion_service = Arc::clone(&service);
    let completion_message_id = assistant.id.clone();
    let completed = tauri::async_runtime::spawn_blocking(move || {
        completion_service.complete_assistant(&completion_message_id, &content)
    })
    .await
    .map_err(|error| format!("Assistant completion task failed: {error}"))?;
    let completed = match completed {
        Ok(message) => message,
        Err(error) => {
            return Err(fail_stream(
                Arc::clone(&service),
                &on_event,
                &assistant.conversation_id,
                &assistant.id,
                error.to_string(),
            )
            .await);
        }
    };

    let _ = on_event.send(ChatEvent::AssistantCompleted { message: completed });
    Ok(accepted)
}

async fn fail_stream(
    service: Arc<ChatService>,
    on_event: &Channel<ChatEvent>,
    conversation_id: &str,
    message_id: &str,
    message: String,
) -> String {
    let persisted_message_id = message_id.to_owned();
    let persisted_error = message.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        service.fail_assistant(&persisted_message_id, &persisted_error)
    })
    .await;
    let _ = on_event.send(ChatEvent::Failed {
        conversation_id: conversation_id.to_owned(),
        message_id: Some(message_id.to_owned()),
        message: message.clone(),
    });
    message
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
    use std::{fs, sync::Arc};

    use uuid::Uuid;

    use super::{
        conversation_title, repository::ChatRepository, ChatEvent, ChatService, StreamingService,
        SubmitMessageInput, TestTextSource,
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
            sequence: 7,
            delta: "hello".to_owned(),
        })
        .expect("chat event should serialize");

        assert_eq!(event["kind"], "assistantDelta");
        assert_eq!(event["conversationId"], "conversation-123");
        assert_eq!(event["messageId"], "message-123");
        assert_eq!(event["sequence"], 7);
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
            let service = ChatService::new(
                ChatRepository::open(&database_path).expect("chat repository should open"),
                StreamingService::new(Arc::new(TestTextSource::default())),
            );
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

            let assistant = service
                .begin_assistant(&accepted.conversation.id)
                .expect("assistant message should begin");
            let completed = service
                .complete_assistant(&assistant.id, "A persisted streamed response.")
                .expect("assistant message should complete");

            let thread = service
                .get_thread(&accepted.conversation.id)
                .expect("completed thread should reload");
            assert_eq!(thread.messages.len(), 2);
            assert_eq!(thread.messages[0].content, accepted.message.content);
            assert_eq!(thread.messages[0].role, "user");
            assert_eq!(thread.messages[0].status, "completed");
            assert_eq!(thread.messages[1].id, completed.id);
            assert_eq!(thread.messages[1].role, "assistant");
            assert_eq!(thread.messages[1].status, "completed");
            assert_eq!(thread.messages[1].content, "A persisted streamed response.");

            let interrupted = service
                .begin_assistant(&accepted.conversation.id)
                .expect("a second assistant message should begin");
            assert_eq!(
                service
                    .repository
                    .fail_interrupted_streams(interrupted.created_at + 1)
                    .expect("interrupted streams should recover"),
                1
            );
            let recovered = service
                .get_thread(&accepted.conversation.id)
                .expect("recovered thread should reload");
            assert_eq!(recovered.messages[2].status, "failed");
            assert_eq!(
                recovered.messages[2].error_message.as_deref(),
                Some("Response interrupted before completion.")
            );
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
