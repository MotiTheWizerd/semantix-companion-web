pub(crate) mod repository;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use repository::{ChatRepository, CommitUserMessage};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{
    app_error::AppError,
    companions::{Companion, CompanionResolver},
    credentials::unix_timestamp_ms,
    inference::{
        InferenceDelta, InferenceExecution, InferenceGateway, InferenceMessage, InferenceRequest,
        ModelTarget, ProviderCredential, Role, ToolCall, ToolRunner,
    },
    models::ModelResolver,
    preferences::{PreferenceRepository, ResolvedVoice},
    streaming::{StreamError, StreamEvent, StreamSink, StreamingService},
    tools::{self, ToolContext},
};

/// Ceiling on execute-and-continue rounds per submission — a runaway model
/// stops re-calling tools after this many, its last text standing as the reply.
const MAX_TOOL_ROUNDS: usize = 4;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Conversation {
    pub(crate) id: String,
    pub(crate) title: String,
    /// Who this thread talks to. The companion carries the model and the
    /// memory, so the conversation itself holds neither.
    pub(crate) companion_id: Option<String>,
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
    /// The sleep ledger stamp — set once this message has been distilled into
    /// long-term memory; the next /sleep skips it.
    pub(crate) slept_at: Option<i64>,
    /// Images sent with this message. Stored beside the text (their own
    /// table), re-injected into the canonical history, rendered in the thread.
    pub(crate) attachments: Vec<MessageAttachment>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MessageAttachment {
    pub(crate) id: String,
    pub(crate) media_type: String,
    /// Base64, no data-URL prefix — the renderer and the provider mapping
    /// each add their own.
    pub(crate) data: String,
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
    /// The companion picked in the composer. None falls back to the thread's
    /// stored companion, then to the built-in one.
    #[serde(default)]
    companion_id: Option<String>,
    content: String,
    /// Recalled memory + time blocks composed by the frontend's pre-send
    /// reflexes. Rides the inference request as a leading system message —
    /// never persisted, so the stored conversation stays clean of injections.
    #[serde(default)]
    memory_context: Option<String>,
    /// The memory agent whose store backs the `recall_memory` tool. None =
    /// memory off for this send, so the tool is never declared.
    #[serde(default)]
    memory_agent_id: Option<String>,
    /// Images riding with this message — already downscaled by the composer.
    #[serde(default)]
    attachments: Vec<AttachmentInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentInput {
    media_type: String,
    /// Base64, no data-URL prefix.
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateConversationCompanionInput {
    conversation_id: String,
    companion_id: String,
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
    /// One tool call's lifecycle on the assistant message — emitted with
    /// status "running", then again with "ok" or "error". Instrument data
    /// only, like the 🧠 chip: runtime-held, never persisted.
    ToolCall {
        #[serde(rename = "conversationId")]
        conversation_id: String,
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "callId")]
        call_id: String,
        name: String,
        arguments: String,
        status: &'static str,
        detail: Option<String>,
    },
}

/// Where a turn's events go.
///
/// ⚑ THE ONE ABSTRACTION THAT LETS A TURN RUN WITHOUT A PERSON. Every event a
/// turn produces used to go to a `Channel` the frontend created for that
/// submission, which quietly meant a turn could only exist because someone
/// pressed enter. A turn nobody asked for has no such channel — it has the
/// app. Both are just somewhere to send events, so the loop takes this and
/// stops caring which it got.
pub(crate) trait ChatEventSink: Send + Sync {
    fn send(&self, event: ChatEvent);
}

/// The human lane: events go to the one window that asked for them.
impl ChatEventSink for Channel<ChatEvent> {
    fn send(&self, event: ChatEvent) {
        let _ = Channel::send(self, event);
    }
}

/// The woken lane: events go app-wide, because nobody is holding a channel
/// open for a turn they did not start. The frontend listens for this and folds
/// the events into whichever thread they name.
pub(crate) struct AppEventSink {
    app: AppHandle,
}

impl AppEventSink {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ChatEventSink for AppEventSink {
    fn send(&self, event: ChatEvent) {
        let _ = self.app.emit(CHAT_EVENT, event);
    }
}

/// App-wide chat events — the woken lane's only way to reach a window.
pub(crate) const CHAT_EVENT: &str = "chat://event";

pub(crate) struct ChatState {
    service: Arc<ChatService>,
}

impl ChatState {
    /// A handle on the turn engine for the waker, which has no `State` to ask.
    pub(crate) fn service(&self) -> Arc<ChatService> {
        Arc::clone(&self.service)
    }

    pub(crate) fn open(database_path: &Path) -> Result<Self, AppError> {
        let repository = ChatRepository::open(database_path)?;
        repository.fail_interrupted_streams(unix_timestamp_ms()?)?;
        Ok(Self {
            service: Arc::new(ChatService::new(
                repository,
                ModelResolver::open(database_path)?,
                CompanionResolver::open(database_path)?,
                PreferenceRepository::open(database_path)?,
                StreamingService::new(Arc::new(InferenceGateway::default())),
                database_path.to_owned(),
            )),
        })
    }
}

pub(crate) struct ChatService {
    repository: ChatRepository,
    model_resolver: ModelResolver,
    companions: CompanionResolver,
    preferences: PreferenceRepository,
    streaming: StreamingService<InferenceExecution, InferenceDelta>,
    /// Handed to ToolContext so the search_conversations tool can open its
    /// own read connection — no contention with the streaming writer.
    database_path: PathBuf,
}

impl ChatService {
    fn new(
        repository: ChatRepository,
        model_resolver: ModelResolver,
        companions: CompanionResolver,
        preferences: PreferenceRepository,
        streaming: StreamingService<InferenceExecution, InferenceDelta>,
        database_path: PathBuf,
    ) -> Self {
        Self {
            repository,
            model_resolver,
            companions,
            preferences,
            streaming,
            database_path,
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

    fn update_companion(
        &self,
        input: UpdateConversationCompanionInput,
    ) -> Result<Conversation, AppError> {
        let conversation_id = input.conversation_id.trim();
        if conversation_id.is_empty() {
            return Err(AppError::validation("Choose an existing conversation."));
        }
        // Resolving first turns a stale id into the built-in companion rather
        // than writing a dangling reference the thread would trip over later.
        let companion = self.companions.resolve(Some(&input.companion_id))?;
        self.repository
            .update_companion(conversation_id, &companion.id)
    }

    fn submit(
        &self,
        mut input: SubmitMessageInput,
        role: &str,
    ) -> Result<PreparedSubmission, AppError> {
        let attachments = accept_attachments(std::mem::take(&mut input.attachments))?;
        let content = input.content.trim();
        if content.is_empty() && attachments.is_empty() {
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
        // Who answers decides what answers: the composer picks a companion, and
        // the companion's own model preference resolves to the actual target.
        let stored_companion_id = conversation_id
            .and_then(|id| self.repository.get_thread(id).ok().flatten())
            .and_then(|thread| thread.conversation.companion_id);
        let companion = self.companions.resolve(
            input
                .companion_id
                .as_deref()
                .or(stored_companion_id.as_deref()),
        )?;
        let voice = self
            .preferences
            .resolve_voice(&companion.model_preference)?;
        let (target, credential) = match voice {
            ResolvedVoice::Configured(configured_model_id) => {
                let model = self.model_resolver.resolve(&configured_model_id)?;
                (
                    ModelTarget {
                        provider_id: model.provider_id,
                        model_id: model.model_id,
                    },
                    ProviderCredential::ApiKey(model.api_key),
                )
            }
            // Claude Code carries its own login — the sidecar authenticates
            // with the user's local Claude session, no stored key involved.
            ResolvedVoice::ClaudeCode(model_id) => (
                ModelTarget {
                    provider_id: "claude_code".to_owned(),
                    model_id,
                },
                ProviderCredential::None,
            ),
            ResolvedVoice::TestStream => (
                ModelTarget {
                    provider_id: "test".to_owned(),
                    model_id: "test-stream".to_owned(),
                },
                ProviderCredential::None,
            ),
        };
        let timestamp = unix_timestamp_ms()?;
        let new_conversation_id = Uuid::new_v4().to_string();
        let message_id = Uuid::new_v4().to_string();
        let title = conversation_title(content);
        let accepted = self.repository.commit_user_message(CommitUserMessage {
            conversation_id,
            role,
            companion_id: &companion.id,
            content,
            title: &title,
            timestamp,
            new_conversation_id: &new_conversation_id,
            message_id: &message_id,
            attachments: &attachments,
        })?;
        let thread = self
            .repository
            .get_thread(&accepted.conversation.id)?
            .ok_or_else(|| AppError::internal("the accepted conversation could not be reloaded"))?;
        let mut messages = canonical_messages(&thread.messages);
        if let Some(memory_context) = input
            .memory_context
            .as_deref()
            .map(str::trim)
            .filter(|context| !context.is_empty())
        {
            messages.insert(0, InferenceMessage::text(Role::System, memory_context.to_owned()));
        }
        // The name goes FIRST, ahead of recalled memory: a companion should
        // know who it is before it is handed what it remembers. Like the memory
        // block, this rides the request only and is never persisted.
        if let Some(identity) = companion_identity(&companion) {
            messages.insert(0, InferenceMessage::text(Role::System, identity));
        }

        let memory_agent_id = input
            .memory_agent_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);

        let tool_context = ToolContext {
            // Resolved HERE, where the companion is already known, and never
            // in the tool loop: the backend is a property of who is answering,
            // not of what the model asked for.
            memory_target: memory_agent_id.as_deref().map(|agent_id| {
                if companion.is_origin {
                    crate::memory::MemoryTarget::Muninn {
                        channel: companion.memory_agent_name.clone(),
                    }
                } else {
                    crate::memory::MemoryTarget::Organ { agent_id: agent_id.to_owned() }
                }
            }),
            memory_agent_id,
            database_path: Some(self.database_path.clone()),
            conversation_id: Some(accepted.conversation.id.clone()),
            serpapi_api_key: std::env::var("SERPAPI_API_KEY")
                .ok()
                .map(|key| key.trim().to_owned())
                .filter(|key| !key.is_empty()),
            // Re-canonicalised at every submission: if the folder vanished or
            // moved since it was picked, the file tools silently stand down
            // rather than run against a stale path.
            workspace_dir: companion
                .workspace_dir
                .as_deref()
                .and_then(|path| std::fs::canonicalize(path).ok())
                .filter(|path| path.is_dir()),
            // From the RESOLVED companion, never from the model's arguments —
            // this is the return address on everything it sends, and a sender
            // it could choose would not be a sender at all.
            companion_id: Some(companion.id.clone()),
        };

        let conversation_session_id = accepted.conversation.id.clone();
        Ok(PreparedSubmission {
            provider_id: target.provider_id.clone(),
            model_id: target.model_id.clone(),
            accepted,
            execution: InferenceExecution {
                request: InferenceRequest {
                    id: Uuid::new_v4().to_string(),
                    target,
                    messages,
                    tools: tools::declarations(&tool_context),
                    session_id: Some(conversation_session_id),
                },
                credential,
                // Attached once the assistant message exists, so tool cards
                // can name the message they belong to.
                tool_runner: None,
            },
            tool_context,
        })
    }

    /// Prepare a turn nobody asked for.
    ///
    /// ⚑ THE NOTICE IS A `system` MESSAGE, NOT A `user` ONE. A woken turn needs
    /// something to answer, and the tempting shortcut is to write a user
    /// message saying "you have a call waiting" — which would put words in the
    /// user's mouth, in their own transcript, forever. The companion is told by
    /// the system, which is what actually happened.
    ///
    /// It lands in the companion's most recent thread, or a new one. A person
    /// looking for what their companion did while they were away should find it
    /// where they last left that companion, not in a hidden place.
    pub(crate) fn prepare_woken(
        &self,
        companion_id: &str,
        notice: String,
    ) -> Result<PreparedSubmission, AppError> {
        // ⚑ NO FALLBACK IN THIS LANE, AND THIS CHECK IS LOAD-BEARING.
        // `resolve` answers an unknown id with the BUILT-IN companion, which is
        // right for a composer whose selection went stale and catastrophic
        // here: a call addressed to an agent that is not on this machine would
        // wake the built-in one, and Rook would answer his own call believing
        // it came from someone else. An id we cannot place is not ours to wake.
        if !self.companions.exists(companion_id)? {
            return Err(AppError::validation(
                "that agent id is not a companion on this machine — nothing to wake",
            ));
        }

        let conversation_id = self
            .repository
            .latest_conversation_for_companion(companion_id)?;
        let mut prepared = self.submit(
            SubmitMessageInput {
                conversation_id,
                companion_id: Some(companion_id.to_owned()),
                content: notice.clone(),
                // No recalled memory and no memory agent: the frontend's
                // pre-send reflexes are a property of a person sending, and
                // there is no frontend in this lane. The companion still has
                // its tools, which is what it needs to answer a call.
                memory_context: None,
                memory_agent_id: None,
                attachments: Vec::new(),
            },
            "system",
        )?;

        // ⚑ THE WOKEN TURN DOES NOT INHERIT THE CONVERSATION'S HISTORY, AND
        // THIS IS THE LINE THAT MAKES THE FEATURE WORK AT ALL.
        //
        // Found by driving it (s502): the first working wake landed in a thread
        // with thirty-nine turns of the companion talking to its user, and the
        // companion answered the notice CONVERSATIONALLY — "I'm here, ready" —
        // addressing the user by name, never touching read_call. One system
        // line asking it to answer a call is a whisper against forty turns of
        // evidence about what it is doing and who it is speaking to.
        //
        // A wake is not the next turn of that conversation. It is a different
        // errand that happens to be recorded there, so the request carries only
        // who it is and what it was woken for. The exchange still PERSISTS into
        // the thread — the user must be able to see what their companion did —
        // but the model is not asked to continue a talk nobody is having.
        //
        // It is also drastically cheaper: forty messages resent per wake, for
        // context that actively misleads, was the worst of both.
        let mut messages = Vec::new();
        if let Some(identity) = self
            .companions
            .resolve(Some(companion_id))
            .ok()
            .as_ref()
            .and_then(companion_identity)
        {
            messages.push(InferenceMessage::text(Role::System, identity));
        }
        // ⚑ THE NOTICE RIDES AS A **USER** MESSAGE, THOUGH IT IS PERSISTED AS A
        // SYSTEM ONE. Not a contradiction — the two roles answer two different
        // questions.
        //
        // In the TRANSCRIPT the role is a claim about who spoke, and writing
        // this as a user message would put words in Moti's mouth in his own
        // history. It stays `system` there, permanently.
        //
        // In the REQUEST the role is a protocol slot, and Claude Code rejects a
        // turn outright without a user message: "A Claude Code turn needs a
        // user message to answer" (observed live, s502 — the turn failed at
        // sequence 43 with an empty body). A request of nothing but system
        // messages is not a request that provider will answer.
        //
        // Honesty is carried by the TEXT rather than the role: the notice's
        // first line says the user did not send it and is not reading it.
        messages.push(InferenceMessage::text(Role::User, notice));
        prepared.execution.request.messages = messages;

        Ok(prepared)
    }

    fn begin_assistant(
        &self,
        conversation_id: &str,
        provider_id: &str,
        model_id: &str,
    ) -> Result<Message, AppError> {
        self.repository.begin_assistant_message(
            conversation_id,
            &Uuid::new_v4().to_string(),
            provider_id,
            model_id,
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

pub(crate) struct PreparedSubmission {
    accepted: AcceptedMessage,
    execution: InferenceExecution,
    provider_id: String,
    model_id: String,
    tool_context: ToolContext,
}

/// Executes one tool and narrates it to the UI. The single place a tool call
/// becomes a tool result, whichever lane asked for it.
struct ChatToolRunner {
    context: ToolContext,
    on_event: Arc<dyn ChatEventSink>,
    conversation_id: String,
    message_id: String,
}

#[async_trait::async_trait]
impl ToolRunner for ChatToolRunner {
    async fn run(&self, call: &ToolCall) -> Result<String, String> {
        self.on_event.send(ChatEvent::ToolCall {
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            call_id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            status: "running",
            detail: None,
        });
        let result = tools::execute(call, &self.context).await;
        self.on_event.send(ChatEvent::ToolCall {
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            call_id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
            status: if result.is_ok() { "ok" } else { "error" },
            detail: result.as_ref().err().cloned(),
        });
        result
    }
}

struct ChatStreamAdapter {
    assistant: Message,
    conversation_id: String,
    message_id: String,
    on_event: Arc<dyn ChatEventSink>,
    content: Mutex<String>,
    /// Tool calls the model requested in the round currently streaming;
    /// drained by the chat loop between rounds.
    tool_calls: Mutex<Vec<ToolCall>>,
}

impl ChatStreamAdapter {
    fn new(assistant: Message, on_event: Arc<dyn ChatEventSink>) -> Self {
        Self {
            conversation_id: assistant.conversation_id.clone(),
            message_id: assistant.id.clone(),
            assistant,
            on_event,
            content: Mutex::new(String::new()),
            tool_calls: Mutex::new(Vec::new()),
        }
    }

    fn content(&self) -> Result<String, StreamError> {
        self.content
            .lock()
            .map(|content| content.clone())
            .map_err(|_| StreamError::new("the chat stream buffer was poisoned"))
    }

    fn take_tool_calls(&self) -> Result<Vec<ToolCall>, StreamError> {
        self.tool_calls
            .lock()
            .map(|mut calls| std::mem::take(&mut *calls))
            .map_err(|_| StreamError::new("the tool call buffer was poisoned"))
    }

    /// A model often stops mid-sentence to call a tool, and the next round's
    /// text would glue straight onto it ("…locked in for theCarved." — s486).
    /// Before a continuation round, close the seam with a paragraph break —
    /// pushed into the buffer AND emitted as a delta, so the live view and the
    /// persisted message stay byte-identical.
    fn separate_rounds(&self) -> Result<(), StreamError> {
        let mut content = self
            .content
            .lock()
            .map_err(|_| StreamError::new("the chat stream buffer was poisoned"))?;
        if content.is_empty() || content.ends_with("\n\n") {
            return Ok(());
        }
        let separator = if content.ends_with('\n') { "\n" } else { "\n\n" };
        content.push_str(separator);
        drop(content);
        self.on_event.send(ChatEvent::AssistantDelta {
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            sequence: 0,
            delta: separator.to_owned(),
        });
        Ok(())
    }
}

impl StreamSink<InferenceDelta> for ChatStreamAdapter {
    fn emit(&self, event: StreamEvent<InferenceDelta>) -> Result<(), StreamError> {
        match event {
            StreamEvent::Started => {
                self.on_event.send(ChatEvent::AssistantStarted {
                    message: self.assistant.clone(),
                });
            }
            StreamEvent::Delta { sequence, payload } => match payload {
                InferenceDelta::Text { text } => {
                    self.content
                        .lock()
                        .map_err(|_| StreamError::new("the chat stream buffer was poisoned"))?
                        .push_str(&text);
                    self.on_event.send(ChatEvent::AssistantDelta {
                        conversation_id: self.conversation_id.clone(),
                        message_id: self.message_id.clone(),
                        sequence,
                        delta: text,
                    });
                }
                InferenceDelta::ToolCall(call) => {
                    self.tool_calls
                        .lock()
                        .map_err(|_| StreamError::new("the tool call buffer was poisoned"))?
                        .push(call);
                }
                InferenceDelta::Reasoning { .. }
                | InferenceDelta::Usage(_)
                | InferenceDelta::Finish(_) => {}
            },
            StreamEvent::Completed | StreamEvent::Failed { .. } => {}
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
pub(crate) async fn update_conversation_companion(
    state: State<'_, ChatState>,
    input: UpdateConversationCompanionInput,
) -> Result<Conversation, String> {
    let service = Arc::clone(&state.service);
    tauri::async_runtime::spawn_blocking(move || service.update_companion(input))
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
    let prepared = tauri::async_runtime::spawn_blocking(move || submit_service.submit(input, "user"))
        .await
        .map_err(|error| format!("Message task failed: {error}"))?
        .map_err(String::from)?;
    drive_turn(service, prepared, Arc::new(on_event)).await
}

/// One turn, start to finish, for whoever asked for it.
///
/// ⚑ THIS USED TO BE THE BODY OF `submit_message`, AND THAT WAS THE WHOLE
/// PROBLEM. The loop lived inside a Tauri command, so the only thing in the
/// process that could start a turn was a person pressing enter. Lifting it out
/// changes nothing for the human lane — `submit_message` is now four lines and
/// a call — and it is the entire reason a companion can be woken by something
/// other than its user.
pub(crate) async fn drive_turn(
    service: Arc<ChatService>,
    prepared: PreparedSubmission,
    on_event: Arc<dyn ChatEventSink>,
) -> Result<AcceptedMessage, String> {
    let PreparedSubmission {
        accepted,
        mut execution,
        provider_id,
        model_id,
        tool_context,
    } = prepared;

    on_event.send(ChatEvent::Accepted {
        conversation: accepted.conversation.clone(),
        message: accepted.message.clone(),
    });

    let conversation_id = accepted.conversation.id.clone();
    let assistant_service = Arc::clone(&service);
    let assistant_conversation_id = conversation_id.clone();
    let assistant = match tauri::async_runtime::spawn_blocking(move || {
        assistant_service.begin_assistant(&assistant_conversation_id, &provider_id, &model_id)
    })
    .await
    .map_err(|error| format!("Assistant task failed: {error}"))?
    {
        Ok(message) => message,
        Err(error) => {
            let message = error.to_string();
            on_event.send(ChatEvent::Failed {
                conversation_id,
                message_id: None,
                message: message.clone(),
            });
            return Err(message);
        }
    };

    let adapter = ChatStreamAdapter::new(assistant.clone(), Arc::clone(&on_event));

    // One executor, both lanes: the chat loop calls it between rounds, and a
    // provider that owns its own agentic loop (Claude Code) calls it
    // mid-stream through the execution. Either way a tool runs in exactly one
    // place and lights the same UI card.
    let tool_runner = Arc::new(ChatToolRunner {
        context: tool_context.clone(),
        on_event: Arc::clone(&on_event),
        conversation_id: assistant.conversation_id.clone(),
        message_id: assistant.id.clone(),
    });
    execution.tool_runner = Some(Arc::clone(&tool_runner) as Arc<dyn ToolRunner>);

    // The execute-and-continue loop: stream a round; if the model requested
    // tools, run them backend-side, fold the results into the request, and
    // stream again. Text keeps landing on the SAME assistant message, so the
    // UI sees one continuous reply.
    let mut rounds = 0;
    loop {
        let round_start = adapter.content().map(|content| content.len());
        let round_start = match round_start {
            Ok(length) => length,
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

        if let Err(error) = service.streaming.stream(&execution, &adapter).await {
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

        let calls = match adapter.take_tool_calls() {
            Ok(calls) => calls,
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
        if calls.is_empty() || rounds >= MAX_TOOL_ROUNDS {
            break;
        }
        rounds += 1;

        let round_text = adapter
            .content()
            .map(|content| content[round_start..].to_owned())
            .unwrap_or_default();
        execution
            .request
            .messages
            .push(InferenceMessage::assistant_tool_calls(round_text, calls.clone()));

        for call in calls {
            let result = tool_runner.run(&call).await;
            // A failed tool becomes a result the model reads and recovers
            // from — it never kills the stream.
            let text = result.unwrap_or_else(|error| format!("Tool error: {error}"));
            execution
                .request
                .messages
                .push(InferenceMessage::tool_result(call.id, text));
        }

        // Seam between this round's text and the continuation round's.
        if let Err(error) = adapter.separate_rounds() {
            return Err(fail_stream(
                Arc::clone(&service),
                &on_event,
                &assistant.conversation_id,
                &assistant.id,
                error.to_string(),
            )
            .await);
        }
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

    on_event.send(ChatEvent::AssistantCompleted { message: completed });
    Ok(accepted)
}

async fn fail_stream(
    service: Arc<ChatService>,
    on_event: &Arc<dyn ChatEventSink>,
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
    on_event.send(ChatEvent::Failed {
        conversation_id: conversation_id.to_owned(),
        message_id: Some(message_id.to_owned()),
        message: message.clone(),
    });
    message
}

/// What a named companion is told about its own name.
///
/// An unnamed companion gets NOTHING — no placeholder, no "you have no name".
/// Silence is the honest state there, and inventing a line about namelessness
/// would tell the model something the user never said.
fn companion_identity(companion: &Companion) -> Option<String> {
    let name = companion.name.as_deref()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(format!("The user prefers to call you {name}."))
}

fn canonical_messages(messages: &[Message]) -> Vec<InferenceMessage> {
    messages
        .iter()
        .filter(|message| message.status == "completed")
        .filter_map(|message| {
            let role = match message.role.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            // Images re-inject on every turn, so the model keeps seeing them
            // for the life of the thread, not just the turn they arrived on.
            if role == Role::User && !message.attachments.is_empty() {
                return Some(InferenceMessage::user_with_images(
                    message.content.clone(),
                    message
                        .attachments
                        .iter()
                        .map(|attachment| {
                            (attachment.media_type.clone(), attachment.data.clone())
                        }),
                ));
            }
            Some(InferenceMessage::text(role, message.content.clone()))
        })
        .collect()
}

const MAX_ATTACHMENTS_PER_MESSAGE: usize = 4;
/// Base64 ceiling per image (~6MB decoded) — the composer downscales far
/// below this; the cap is the backstop, not the budget.
const MAX_ATTACHMENT_BASE64_BYTES: usize = 8 * 1024 * 1024;
const ACCEPTED_IMAGE_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Validate the composer's images and mint their identities.
fn accept_attachments(
    attachments: Vec<AttachmentInput>,
) -> Result<Vec<MessageAttachment>, AppError> {
    if attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(AppError::validation(format!(
            "A message can carry up to {MAX_ATTACHMENTS_PER_MESSAGE} images."
        )));
    }
    attachments
        .into_iter()
        .map(|attachment| {
            let media_type = attachment.media_type.trim().to_ascii_lowercase();
            if !ACCEPTED_IMAGE_TYPES.contains(&media_type.as_str()) {
                return Err(AppError::validation(
                    "Only PNG, JPEG, WebP and GIF images can be attached.",
                ));
            }
            let data = attachment.data.trim().to_owned();
            if data.is_empty() {
                return Err(AppError::validation("An attached image arrived empty."));
            }
            if data.len() > MAX_ATTACHMENT_BASE64_BYTES {
                return Err(AppError::validation(
                    "An attached image is too large — images must be under ~6MB.",
                ));
            }
            Ok(MessageAttachment {
                id: Uuid::new_v4().to_string(),
                media_type,
                data,
            })
        })
        .collect()
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
        companion_identity, conversation_title, repository::ChatRepository, ChatEvent, ChatService,
        SubmitMessageInput,
    };
    use crate::{
        companions::{Companion, CompanionResolver},
        database,
        inference::InferenceGateway,
        models::ModelResolver,
        preferences::{ModelPreference, PreferenceRepository},
        streaming::StreamingService,
    };

    #[test]
    fn a_named_companion_is_told_its_name_and_an_unnamed_one_is_told_nothing() {
        let companion = |name: Option<&str>| Companion {
            id: "companion-1".to_owned(),
            name: name.map(str::to_owned),
            memory_agent_name: "companion-1-memory".to_owned(),
            model_preference: ModelPreference::Inherit,
            is_built_in: false,
            created_at: 1,
            updated_at: 1,
            workspace_dir: None,
            is_origin: false,
        };

        assert_eq!(
            companion_identity(&companion(Some("Ragnar"))).as_deref(),
            Some("The user prefers to call you Ragnar.")
        );
        assert_eq!(companion_identity(&companion(None)), None);
        assert_eq!(
            companion_identity(&companion(Some("   "))),
            None,
            "a blank name says nothing rather than saying something empty"
        );
    }

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
                ModelResolver::open(&database_path).expect("model resolver should open"),
                CompanionResolver::open(&database_path).expect("companion resolver should open"),
                PreferenceRepository::open(&database_path)
                    .expect("preference repository should open"),
                StreamingService::new(Arc::new(InferenceGateway::default())),
                database_path.clone(),
            );
            let accepted = service
                .submit(
                    SubmitMessageInput {
                        conversation_id: None,
                        companion_id: None,
                        content: "Remember that my favorite ship is the Long Serpent.".to_owned(),
                        memory_context: None,
                        memory_agent_id: None,
                        attachments: Vec::new(),
                    },
                    "user",
                )
                .expect("message should persist");

            let conversations = service
                .list_conversations()
                .expect("conversations should reload");
            assert_eq!(conversations.len(), 1);
            assert_eq!(conversations[0].id, accepted.accepted.conversation.id);
            let built_in_id: String = rusqlite::Connection::open(&database_path)
                .expect("test database should open")
                .query_row("SELECT id FROM companions WHERE is_built_in = 1", [], |row| {
                    row.get(0)
                })
                .expect("the built-in companion should exist");
            assert_eq!(
                conversations[0].companion_id.as_deref(),
                Some(built_in_id.as_str()),
                "a thread sent with no pick belongs to the built-in companion"
            );

            let assistant = service
                .begin_assistant(&accepted.accepted.conversation.id, "test", "test-stream")
                .expect("assistant message should begin");
            let completed = service
                .complete_assistant(&assistant.id, "A persisted streamed response.")
                .expect("assistant message should complete");

            let thread = service
                .get_thread(&accepted.accepted.conversation.id)
                .expect("completed thread should reload");
            assert_eq!(thread.messages.len(), 2);
            assert_eq!(
                thread.messages[0].content,
                accepted.accepted.message.content
            );
            assert_eq!(thread.messages[0].role, "user");
            assert_eq!(thread.messages[0].status, "completed");
            assert_eq!(thread.messages[1].id, completed.id);
            assert_eq!(thread.messages[1].role, "assistant");
            assert_eq!(thread.messages[1].status, "completed");
            assert_eq!(thread.messages[1].content, "A persisted streamed response.");

            let interrupted = service
                .begin_assistant(&accepted.accepted.conversation.id, "test", "test-stream")
                .expect("a second assistant message should begin");
            assert_eq!(
                service
                    .repository
                    .fail_interrupted_streams(interrupted.created_at + 1)
                    .expect("interrupted streams should recover"),
                1
            );
            let recovered = service
                .get_thread(&accepted.accepted.conversation.id)
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
