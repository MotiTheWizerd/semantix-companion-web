//! The Claude Code lane: streams through a Node sidecar running the Claude
//! Agent SDK with `settingSources: []` — the user's local Claude login
//! answers, but none of their global hooks, settings or CLAUDE.md load, so
//! the companion's own memory system stays the only one in the room.
//!
//! Protocol: one JSON object per line, requests down the child's stdin
//! (`query` / `toolResult` / `cancel`), events back up its stdout (`delta` /
//! `toolCallDelta` / `toolCall` / `usage` / `done` / `error`), matched to
//! callers by request id.
//!
//! Finding the sidecar, in order: the `COMPANION_SIDECAR_DIR` env var, then
//! the bundled resource dir registered at startup by `lib.rs`, then — dev
//! builds only — the `sidecar/` directory beside `src-tauri`, baked in at
//! compile time. A packaged app never reaches that last one.

use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{mpsc, Mutex},
};

use crate::{
    inference::{
        capabilities::ProviderCapabilities,
        provider::{InferenceProvider, ProviderCredential, ToolRunner},
        ContentPart, FinishReason, InferenceDelta, InferenceRequest, Role, TokenUsage, ToolCall,
        ToolCallDelta,
    },
    streaming::{DeltaSink, StreamError},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidecarQuery<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    conversation_id: &'a str,
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    transcript: Vec<SidecarTurn<'a>>,
    user_text: String,
    /// The same declarations every other provider gets, handed to the SDK as
    /// in-process MCP tools; Rust still executes them.
    tools: Vec<SidecarTool<'a>>,
    /// Images on the LIVE turn.
    images: Vec<SidecarImage<'a>>,
}

/// One archived turn. It carries its own images now, and the sidecar spends
/// them only when it has no session to resume (s495).
///
/// THE OLD RULE WAS RIGHT FOR THE WRONG REASON. "Earlier turns travel as text,
/// re-sending every past image would re-pay for the whole album on every
/// message" — true, and the sidecar already solved it: a live conversation
/// RESUMES its Claude session, which still holds every image sent on the turn
/// it arrived. The transcript is not the running context. It is the COLD
/// RE-SEED, read on the first turn after the app restarts and never again.
///
/// So the album is not re-paid per message; it is paid once, on the one turn
/// where the alternative is a model told in words that it was shown something.
/// And the second half is not a cost question at all: an image-only turn
/// flattened to text produced an EMPTY string, which the filter below then
/// dropped — so the turn did not lose its picture, it disappeared, leaving a
/// hole in the record that nothing announced.
#[derive(Serialize)]
struct SidecarTurn<'a> {
    role: &'static str,
    text: String,
    images: Vec<SidecarImage<'a>>,
}

#[derive(Serialize)]
struct SidecarTool<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidecarImage<'a> {
    media_type: &'a str,
    /// Base64, no data-URL prefix — the sidecar wraps it in a content block.
    data: &'a str,
}

/// A tool result travelling back down to the sidecar, which resolves the
/// waiting MCP handler with it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SidecarToolResult<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// The host stopped listening to a query; the sidecar should stop generating.
#[derive(Serialize)]
struct SidecarCancel<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    id: &'a str,
}

/// Tells the sidecar to abandon a query the host stopped reading. Armed for
/// the life of a stream; dropped ARMED means the stream future was cancelled
/// mid-turn (the composer's stop), and the sidecar would otherwise keep
/// generating — and paying — for an answer nobody will hear. Disarmed on
/// every ordinary exit, where the query is already over. Either way, the
/// drop clears the routing entry, so a cancelled stream leaks nothing.
struct CancelOnDrop {
    link: Arc<SidecarLink>,
    id: String,
    armed: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.link.pending.lock() {
            pending.remove(&self.id);
        }
        if !self.armed {
            return;
        }
        let link = Arc::clone(&self.link);
        let id = std::mem::take(&mut self.id);
        tauri::async_runtime::spawn(async move { link.cancel(&id).await });
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum SidecarEvent {
    Delta {
        text: String,
    },
    Reasoning {
        text: String,
    },
    /// Claude's native partial-input event, normalized by the sidecar before
    /// it crosses into the provider-independent inference stream.
    #[serde(rename_all = "camelCase")]
    ToolCallDelta {
        call_id: String,
        name: String,
        arguments_delta: String,
    },
    /// Claude asked for one of OUR tools; Rust runs it and answers.
    #[serde(rename_all = "camelCase")]
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    #[serde(rename_all = "camelCase")]
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Done,
    Error {
        message: String,
    },
}

#[derive(Debug, Deserialize)]
struct SidecarLine {
    id: String,
    #[serde(flatten)]
    event: SidecarEvent,
}

type PendingMap = Arc<StdMutex<HashMap<String, mpsc::UnboundedSender<SidecarEvent>>>>;

struct SidecarHandle {
    child: Child,
    stdin: ChildStdin,
}

/// The pipe to the sidecar: the child and its stdin, plus the routing table
/// from request id to the stream waiting on it. Shared, so a stream being
/// dropped mid-turn can still reach the child to say so.
struct SidecarLink {
    handle: Mutex<Option<SidecarHandle>>,
    pending: PendingMap,
}

pub(crate) struct ClaudeProvider {
    link: Arc<SidecarLink>,
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        Self {
            link: Arc::new(SidecarLink {
                handle: Mutex::new(None),
                pending: Arc::new(StdMutex::new(HashMap::new())),
            }),
        }
    }
}

/// The sidecar shipped inside the app bundle. Registered once at startup,
/// because the resource dir is only knowable from the Tauri `AppHandle` and
/// the gateway that builds this provider has no handle to thread down.
static BUNDLED_SIDECAR_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Tell the Claude lane where the packaged sidecar lives. Called once from
/// `lib.rs` setup; a second call is ignored rather than panicking, since a
/// re-registration is never the interesting failure.
pub(crate) fn set_bundled_sidecar_dir(dir: PathBuf) {
    let _ = BUNDLED_SIDECAR_DIR.set(dir);
}

/// Where the sidecar lives, in preference order. Only the dev fallback is
/// compile-time — a packaged build resolves against the bundle, and the env
/// var wins everywhere for the times a machine disagrees with both.
fn sidecar_dir() -> Result<PathBuf, StreamError> {
    let env_override = std::env::var("COMPANION_SIDECAR_DIR")
        .ok()
        .filter(|dir| !dir.trim().is_empty())
        .map(PathBuf::from);
    let candidates = [
        env_override,
        BUNDLED_SIDECAR_DIR.get().cloned(),
        // Dev only: the repo's `sidecar/` beside `src-tauri`. On any other
        // machine this path does not exist, which is the point.
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|candidate| usable_sidecar(&candidate))
        .ok_or_else(|| {
            StreamError::new(
                "The Claude Code sidecar could not be found. Reinstalling Semantix Companion \
                 should restore it, or set COMPANION_SIDECAR_DIR to point at it.",
            )
        })
}

/// A candidate counts only if it resolves AND carries an `index.mjs` — an
/// empty directory left behind by a half-finished install is not a sidecar.
fn usable_sidecar(candidate: &Path) -> Option<PathBuf> {
    let dir = candidate.canonicalize().ok()?;
    dir.join("index.mjs").is_file().then_some(dir)
}

impl SidecarLink {
    /// Tell the sidecar to abandon `id`. Only a LIVE sidecar is told — a dead
    /// one has nothing to abandon, and respawning it to say so would be absurd.
    async fn cancel(&self, id: &str) {
        let Ok(line) = serde_json::to_string(&SidecarCancel { kind: "cancel", id }) else {
            return;
        };
        let mut guard = self.handle.lock().await;
        let Some(handle) = guard.as_mut() else {
            return;
        };
        if handle.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = handle.stdin.write_all(format!("{line}\n").as_bytes()).await;
    }

    /// Write one protocol line to the sidecar, spawning (or respawning after
    /// a crash) as needed.
    async fn write_line(&self, line: String) -> Result<(), StreamError> {
        let mut guard = self.handle.lock().await;
        let needs_spawn = match guard.as_mut() {
            None => true,
            // try_wait() = has the child exited? A dead sidecar is replaced,
            // not mourned.
            Some(handle) => handle.child.try_wait().ok().flatten().is_some(),
        };
        if needs_spawn {
            *guard = Some(self.spawn().await?);
        }
        let handle = guard
            .as_mut()
            .ok_or_else(|| StreamError::new("the sidecar handle vanished mid-spawn"))?;
        handle.stdin.write_all(line.as_bytes()).await.map_err(|error| {
            StreamError::new(format!("Could not reach the Claude Code sidecar: {error}"))
        })
    }

    async fn spawn(&self) -> Result<SidecarHandle, StreamError> {
        let dir = sidecar_dir()?;
        let mut child = Command::new("node")
            .arg("index.mjs")
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| match error.kind() {
                // The one failure a user can actually act on. Node is not an
                // install requirement for Companion — only this lane needs it,
                // so say that plainly instead of leaking an OS error.
                ErrorKind::NotFound => StreamError::new(
                    "The Claude Code lane needs Node.js, and it was not found on this machine. \
                     Install it from nodejs.org, then send the message again — the rest of \
                     Companion works without it.",
                ),
                _ => StreamError::new(format!(
                    "The Claude Code sidecar could not be started: {error}"
                )),
            })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| StreamError::new("the sidecar spawned without a stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| StreamError::new("the sidecar spawned without a stdout pipe"))?;

        // One reader per child: route each stdout line to the waiting stream
        // by request id. On EOF (the child died) every waiter is failed fast
        // instead of hanging until a timeout that doesn't exist.
        let pending = Arc::clone(&self.pending);
        tauri::async_runtime::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(parsed) = serde_json::from_str::<SidecarLine>(&line) else {
                    continue;
                };
                let sender = pending
                    .lock()
                    .ok()
                    .and_then(|map| map.get(&parsed.id).cloned());
                if let Some(sender) = sender {
                    let _ = sender.send(parsed.event);
                }
            }
            if let Ok(map) = pending.lock() {
                for sender in map.values() {
                    let _ = sender.send(SidecarEvent::Error {
                        message: "The Claude Code sidecar stopped unexpectedly.".to_owned(),
                    });
                }
            }
        });

        Ok(SidecarHandle { child, stdin })
    }
}

/// Flatten a message's text parts. Images are pulled out separately (they
/// ride the live turn as content blocks), so this is the words only.
fn message_text(parts: &[ContentPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            ContentPart::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The image parts of a message, in the order they were attached.
fn message_images(parts: &[ContentPart]) -> Vec<SidecarImage<'_>> {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Image { media_type, data } => Some(SidecarImage { media_type, data }),
            ContentPart::Text { .. } => None,
        })
        .collect()
}

/// Split the canonical messages into the sidecar's shape: system text joined
/// into one prompt, the LAST user message as the live turn, and everything
/// between as the prior transcript.
fn build_query<'a>(request: &'a InferenceRequest) -> Result<SidecarQuery<'a>, StreamError> {
    let system_prompt = {
        let joined = request
            .messages
            .iter()
            .filter(|message| message.role == Role::System)
            .map(|message| message_text(&message.content))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        (!joined.is_empty()).then_some(joined)
    };

    let last_user_index = request
        .messages
        .iter()
        .rposition(|message| message.role == Role::User)
        .ok_or_else(|| StreamError::new("A Claude Code turn needs a user message to answer."))?;

    let transcript: Vec<SidecarTurn> = request.messages[..last_user_index]
        .iter()
        .filter_map(|message| {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System | Role::Tool => return None,
            };
            Some(SidecarTurn {
                role,
                text: message_text(&message.content),
                images: message_images(&message.content),
            })
        })
        // A turn earns its place with WORDS OR PICTURES. It used to need words,
        // so a bare screenshot was indistinguishable from a turn that never
        // happened.
        .filter(|turn| !turn.text.is_empty() || !turn.images.is_empty())
        .collect();

    Ok(SidecarQuery {
        id: &request.id,
        kind: "query",
        conversation_id: request.session_id.as_deref().unwrap_or(&request.id),
        model: &request.target.model_id,
        system_prompt,
        transcript,
        user_text: message_text(&request.messages[last_user_index].content),
        images: message_images(&request.messages[last_user_index].content),
        tools: request
            .tools
            .iter()
            .map(|tool| SidecarTool {
                name: &tool.name,
                description: &tool.description,
                parameters: &tool.parameters,
            })
            .collect(),
    })
}

#[async_trait]
impl InferenceProvider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude_code"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            tools: true,
            reasoning: true,
            ..ProviderCapabilities::TEXT_STREAMING
        }
    }

    async fn stream(
        &self,
        request: &InferenceRequest,
        _credential: &ProviderCredential,
        tools: Option<&dyn ToolRunner>,
        sink: &dyn DeltaSink<InferenceDelta>,
    ) -> Result<(), StreamError> {
        let query = build_query(request)?;
        let line = serde_json::to_string(&query)
            .map_err(|error| StreamError::new(format!("sidecar request failed to encode: {error}")))?;

        let (sender, mut receiver) = mpsc::unbounded_channel();
        {
            let mut pending = self
                .link
                .pending
                .lock()
                .map_err(|_| StreamError::new("the sidecar routing table was poisoned"))?;
            pending.insert(request.id.clone(), sender);
        }
        // Deregisters on every exit path — a leaked entry would route a later
        // duplicate id into a dead channel — and, if this future is dropped
        // mid-query, tells the sidecar to stop.
        let mut cancel = CancelOnDrop {
            link: Arc::clone(&self.link),
            id: request.id.clone(),
            armed: true,
        };
        let result = async {
            self.link.write_line(format!("{line}\n")).await?;
            loop {
                let Some(event) = receiver.recv().await else {
                    return Err(StreamError::new(
                        "The Claude Code sidecar dropped this request.",
                    ));
                };
                match event {
                    SidecarEvent::Delta { text } => {
                        sink.emit_delta(InferenceDelta::Text { text })?;
                    }
                    SidecarEvent::Reasoning { text } => {
                        sink.emit_delta(InferenceDelta::Reasoning { text })?;
                    }
                    SidecarEvent::ToolCallDelta {
                        call_id,
                        name,
                        arguments_delta,
                    } => {
                        sink.emit_delta(InferenceDelta::ToolCallDelta(ToolCallDelta {
                            id: call_id,
                            name,
                            arguments_delta,
                        }))?;
                    }
                    // Claude runs the tool inside its own turn, so we execute
                    // now and answer mid-stream rather than surfacing the call
                    // for the chat loop's between-rounds pass.
                    SidecarEvent::ToolCall {
                        call_id,
                        name,
                        arguments,
                    } => {
                        let outcome = match tools {
                            Some(runner) => {
                                runner
                                    .run(&ToolCall {
                                        id: call_id.clone(),
                                        name,
                                        arguments,
                                    })
                                    .await
                            }
                            None => Err("This conversation has no tools available.".to_owned()),
                        };
                        let (content, error) = match outcome {
                            Ok(text) => (Some(text), None),
                            Err(message) => (None, Some(message)),
                        };
                        let reply = serde_json::to_string(&SidecarToolResult {
                            kind: "toolResult",
                            call_id: &call_id,
                            content,
                            error,
                        })
                        .map_err(|error| {
                            StreamError::new(format!("tool result failed to encode: {error}"))
                        })?;
                        self.link.write_line(format!("{reply}\n")).await?;
                    }
                    SidecarEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        sink.emit_delta(InferenceDelta::Usage(TokenUsage {
                            input_tokens,
                            output_tokens,
                            total_tokens: input_tokens + output_tokens,
                        }))?;
                    }
                    SidecarEvent::Done => {
                        sink.emit_delta(InferenceDelta::Finish(FinishReason::Stop))?;
                        return Ok(());
                    }
                    SidecarEvent::Error { message } => {
                        return Err(StreamError::new(message));
                    }
                }
            }
        }
        .await;
        // The query ended on its own terms — done, errored, or the sidecar
        // dropped it. There is nothing left to cancel.
        cancel.armed = false;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{build_query, SidecarEvent, SidecarLine};
    use crate::inference::{InferenceMessage, InferenceRequest, ModelTarget, Role};

    fn request(messages: Vec<InferenceMessage>) -> InferenceRequest {
        InferenceRequest {
            id: "req-1".to_owned(),
            target: ModelTarget {
                provider_id: "claude_code".to_owned(),
                model_id: "opus".to_owned(),
            },
            messages,
            tools: Vec::new(),
            session_id: Some("conversation-9".to_owned()),
        }
    }

    #[test]
    fn the_query_splits_system_history_and_the_live_turn() {
        let full_request = request(vec![
            InferenceMessage::text(Role::System, "You are Rook."),
            InferenceMessage::text(Role::System, "Recalled memory."),
            InferenceMessage::text(Role::User, "First question"),
            InferenceMessage::text(Role::Assistant, "First answer"),
            InferenceMessage::text(Role::User, "Second question"),
        ]);
        let query = build_query(&full_request).expect("a valid request should build");

        assert_eq!(query.conversation_id, "conversation-9");
        assert_eq!(query.model, "opus");
        assert_eq!(
            query.system_prompt.as_deref(),
            Some("You are Rook.\n\nRecalled memory.")
        );
        assert_eq!(query.transcript.len(), 2);
        assert_eq!(query.transcript[0].role, "user");
        assert_eq!(query.transcript[0].text, "First question");
        assert_eq!(query.transcript[1].role, "assistant");
        assert_eq!(query.user_text, "Second question");
    }

    /// The companion's tools must reach the sidecar verbatim — same
    /// declarations every other provider gets, no Claude-specific copy.
    #[test]
    fn tool_declarations_ride_the_query_untouched() {
        let mut full_request = request(vec![InferenceMessage::text(Role::User, "Remember?")]);
        full_request.tools = vec![crate::inference::ToolDeclaration {
            name: "recall_memory".to_owned(),
            description: "Fetch one memory.".to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"],
            }),
        }];

        let query = build_query(&full_request).expect("a valid request should build");
        let encoded = serde_json::to_value(&query).expect("the query should serialize");
        assert_eq!(encoded["tools"][0]["name"], "recall_memory");
        assert_eq!(encoded["tools"][0]["description"], "Fetch one memory.");
        assert_eq!(
            encoded["tools"][0]["parameters"]["properties"]["name"]["type"],
            "string"
        );
    }

    /// Images on the live turn travel as their own field, and the words stay
    /// clean text beside them — never stringified into the prompt.
    #[test]
    fn images_on_the_live_turn_ride_the_query() {
        let full_request = request(vec![
            InferenceMessage::text(Role::User, "An older turn"),
            InferenceMessage::user_with_images(
                "What is in this picture?",
                vec![("image/png".to_owned(), "aGk=".to_owned())],
            ),
        ]);

        let query = build_query(&full_request).expect("a valid request should build");
        assert_eq!(query.user_text, "What is in this picture?");
        assert_eq!(query.images.len(), 1);
        assert_eq!(query.images[0].media_type, "image/png");
        assert_eq!(query.images[0].data, "aGk=");
        // The earlier turn stays in the transcript as words.
        assert_eq!(query.transcript.len(), 1);
        assert_eq!(query.transcript[0].text, "An older turn");
    }

    #[test]
    fn a_request_without_a_user_turn_is_refused() {
        assert!(build_query(&request(vec![InferenceMessage::text(
            Role::System,
            "system only",
        )]))
        .is_err());
    }

    #[test]
    fn sidecar_tool_fragments_decode_into_the_camel_case_wire_contract() {
        let line: SidecarLine = serde_json::from_str(
            r#"{"id":"req-1","event":"toolCallDelta","callId":"tool-7","name":"send_in_call","argumentsDelta":"{\"body\":\"Hi"}"#,
        )
        .expect("the sidecar fragment should decode");
        assert_eq!(line.id, "req-1");
        match line.event {
            SidecarEvent::ToolCallDelta {
                call_id,
                name,
                arguments_delta,
            } => {
                assert_eq!(call_id, "tool-7");
                assert_eq!(name, "send_in_call");
                assert_eq!(arguments_delta, r#"{"body":"Hi"#);
            }
            event => panic!("expected a tool fragment, got {event:?}"),
        }
    }

    #[test]
    fn sidecar_reasoning_decodes_into_the_provider_neutral_stream() {
        let line: SidecarLine = serde_json::from_str(
            r#"{"id":"req-1","event":"reasoning","text":"Checking both workspaces."}"#,
        )
        .expect("the sidecar reasoning event should decode");
        assert_eq!(line.id, "req-1");
        match line.event {
            SidecarEvent::Reasoning { text } => {
                assert_eq!(text, "Checking both workspaces.");
            }
            event => panic!("expected reasoning, got {event:?}"),
        }
    }
}
