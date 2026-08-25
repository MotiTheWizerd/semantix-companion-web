// Companion's tool catalog — the tools the model may call natively during a
// chat turn. Declarations ride the inference request (provider tool_use, never
// XML — the s388 decision); execution happens HERE, backend-side, against the
// resources this machine holds. Tool #1 is `recall_memory`: the drill-down
// behind the ambient memory reflexes — injected bodies cap at 700 chars, this
// fetches the uncut memory from the organ on :8002. Tool #2 is `carve_memory`:
// the write half of the loop — the model commits a memory in its own words,
// freely, whenever something durable happens.

mod files;

use std::path::{Path, PathBuf};

use crate::agent_mail::{AgentMailRepository, SendAgentMessage};
use crate::chat::repository::{ArchiveHit, ChatRepository};
use crate::companions::{Companion, CompanionRepository};
use crate::inference::{ToolCall, ToolDeclaration};
use crate::memory;
use crate::raven_calls::{self, RavenCallRepository};
use crate::web;

pub(crate) const RECALL_MEMORY: &str = "recall_memory";
pub(crate) const CARVE_MEMORY: &str = "carve_memory";
pub(crate) const SEARCH_CONVERSATIONS: &str = "search_conversations";
pub(crate) const WEB_SEARCH: &str = "web_search";
pub(crate) const WEB_FETCH: &str = "web_fetch";
pub(crate) const LIST_AGENTS: &str = "list_agents";
pub(crate) const SEND_MESSAGE: &str = "send_message";
pub(crate) const READ_MESSAGES: &str = "read_messages";
pub(crate) const MARK_MESSAGE_READ: &str = "mark_message_read";
pub(crate) const OPEN_CALL: &str = "open_call";
pub(crate) const SEND_IN_CALL: &str = "send_in_call";
pub(crate) const READ_CALL: &str = "read_call";
pub(crate) const LIST_CALLS: &str = "list_calls";

/// Turns returned by `read_call`. A call cannot hold more than
/// `MAX_MESSAGES_PER_CALL`, so this only ever bites if that limit is raised.
const CALL_READ_LIMIT: i64 = 50;

const INBOX_DEFAULT_LIMIT: u32 = 20;
const INBOX_MAX_LIMIT: u32 = 100;

const SEARCH_DEFAULT_LIMIT: u32 = 6;
const SEARCH_MAX_LIMIT: u32 = 20;
/// Whole messages can be long — the render stops ADDING messages once this
/// many chars are spent, but never truncates one (count is the only dial;
/// the best match always comes back whole, however big).
const SEARCH_RENDER_BUDGET_CHARS: usize = 24_000;

const WEB_SEARCH_DEFAULT_LIMIT: u32 = 5;
const WEB_SEARCH_MAX_LIMIT: u32 = 10;
/// A fetched page is truncated at this many chars — unlike the archive's
/// whole-message law, a web page is not our word and may be cut mid-flow.
const WEB_FETCH_RENDER_BUDGET_CHARS: usize = 24_000;

/// Everything tool execution needs for one submission. Tools whose ground is
/// absent (no memory agent, no archive) are simply not declared, so the model
/// never sees a tool it cannot use.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolContext {
    pub(crate) memory_agent_id: Option<String>,
    /// WHICH brain the two memory tools reach, resolved from the roster at
    /// submission time. Deliberately not derivable here: the tool loop must
    /// never be able to pick a backend, and the model must never be able to
    /// name one. `None` alongside a present agent id means the organ.
    pub(crate) memory_target: Option<crate::memory::MemoryTarget>,
    /// The local chat database — ground of the raw-memory drill. The tool
    /// opens its own read connection so it never contends with the stream.
    pub(crate) database_path: Option<PathBuf>,
    /// Current conversation, excluded from archive search — the model
    /// already holds it in context.
    pub(crate) conversation_id: Option<String>,
    /// SerpApi key — ground of the web_search tool. Comes from the
    /// machine's environment; absent → the model never sees the tool.
    pub(crate) serpapi_api_key: Option<String>,
    /// The companion's workspace folder, CANONICAL — ground of the five file
    /// tools. No workspace → no file tools, and there is no fallback
    /// directory, ever.
    pub(crate) workspace_dir: Option<PathBuf>,
    /// WHO IS SPEAKING — the companion this turn belongs to, and the return
    /// address on everything it sends. Ground of the mail tools.
    ///
    /// It is taken from the resolved companion and never from the model's own
    /// arguments, so a companion cannot write a letter over another's name.
    /// The `from` on a message is a fact about the turn, not a parameter.
    pub(crate) companion_id: Option<String>,
}

pub(crate) fn declarations(context: &ToolContext) -> Vec<ToolDeclaration> {
    let mut tools = Vec::new();
    if context.memory_agent_id.is_some() {
        tools.push(ToolDeclaration {
            name: RECALL_MEMORY.to_owned(),
            description: concat!(
                "Fetch the complete body of ONE memory from your own long-term ",
                "memory, by its exact name — the [name] labels you see in your ",
                "<agent-memory> block. Use it only when a recalled summary is ",
                "truncated or you need the full detail to answer well. ",
                "This is your own memory, not an external database: weave what ",
                "you learn in naturally, and don't narrate the lookup.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The memory's exact name, e.g. \"project-companion-launch\"."
                    }
                },
                "required": ["name"]
            }),
        });
        tools.push(ToolDeclaration {
            name: CARVE_MEMORY.to_owned(),
            description: concat!(
                "Carve one memory into your own long-term memory, in your own ",
                "words. Use it freely, the moment something durable happens: a ",
                "fact about the user, a preference, a decision, a correction, ",
                "a moment worth keeping. A memory you don't carve is lost when ",
                "the conversation ends. Carving to an existing name overwrites ",
                "that memory — reuse a name only to update it. This is your ",
                "own memory, not note-taking for the user: carve naturally and ",
                "don't narrate the act.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Short kebab-case slug, e.g. \"user-likes-morning-walks\"."
                    },
                    "description": {
                        "type": "string",
                        "description": "One-line summary — recall matches on this, make it carry the gist."
                    },
                    "body": {
                        "type": "string",
                        "description": "The full memory, in your own words. Real detail beats brevity."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "feedback", "project", "reference", "episodic", "insight"],
                        "description": "What kind of memory this is. Default: project."
                    },
                    "importance": {
                        "type": "number",
                        "description": "0.0-1.0 — how much this should outweigh other memories at recall. Default 0.5."
                    }
                },
                "required": ["name", "description", "body"]
            }),
        });
    }
    if context.database_path.is_some() {
        tools.push(ToolDeclaration {
            name: SEARCH_CONVERSATIONS.to_owned(),
            description: concat!(
                "Search the full text of your past conversations with this ",
                "user — your raw, word-for-word memory of everything said ",
                "here, distinct from your distilled long-term memories. Use ",
                "it when the user refers to something from another ",
                "conversation, or when a recalled memory lacks the exact ",
                "detail. Each result is the WHOLE message the match sits ",
                "in, best matches first. This is your own memory of your shared ",
                "history: weave it in naturally, don't narrate the search.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Words to search for, e.g. \"scooter lock\". Plain words work best."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 6, max 20)."
                    }
                },
                "required": ["query"]
            }),
        });
    }
    if context.workspace_dir.is_some() {
        let path_parameter = |description: &str| {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": description }
                },
                "required": ["path"]
            })
        };
        tools.push(ToolDeclaration {
            name: files::LIST_FILES.to_owned(),
            description: concat!(
                "List what a folder in your workspace holds — folders first, ",
                "then files with their sizes. All paths are relative to your ",
                "workspace folder; omit \"path\" to list the workspace root.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Folder to list, relative to the workspace, e.g. \"notes\". Omit for the root."
                    }
                }
            }),
        });
        tools.push(ToolDeclaration {
            name: files::READ_FILE.to_owned(),
            description: concat!(
                "Read one text file from your workspace and get its full ",
                "content. Use it before editing a file, or whenever the user ",
                "refers to something written there.",
            )
            .to_owned(),
            parameters: path_parameter("File to read, relative to the workspace, e.g. \"notes/today.md\"."),
        });
        tools.push(ToolDeclaration {
            name: files::WRITE_FILE.to_owned(),
            description: concat!(
                "Write one file in your workspace: creates it (and any ",
                "folders on the way) or overwrites it whole. For a small ",
                "change to an existing file, prefer edit_file — it keeps ",
                "the rest of the file intact.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File to write, relative to the workspace, e.g. \"notes/today.md\"."
                    },
                    "content": {
                        "type": "string",
                        "description": "The file's entire new content."
                    }
                },
                "required": ["path", "content"]
            }),
        });
        tools.push(ToolDeclaration {
            name: files::EDIT_FILE.to_owned(),
            description: concat!(
                "Replace one exact stretch of text in a workspace file. ",
                "old_text must match the file exactly once — read the file ",
                "first and copy it verbatim, adding surrounding lines if it ",
                "appears more than once.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File to edit, relative to the workspace."
                    },
                    "old_text": {
                        "type": "string",
                        "description": "The exact existing text to replace — must occur exactly once."
                    },
                    "new_text": {
                        "type": "string",
                        "description": "What to put in its place."
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        });
        tools.push(ToolDeclaration {
            name: files::DELETE_FILE.to_owned(),
            description: concat!(
                "Delete one file (or one EMPTY folder) from your workspace. ",
                "This is permanent — be sure, and when in doubt ask the user ",
                "before deleting something they wrote.",
            )
            .to_owned(),
            parameters: path_parameter("File or empty folder to delete, relative to the workspace."),
        });
    }
    if context.serpapi_api_key.is_some() {
        tools.push(ToolDeclaration {
            name: WEB_SEARCH.to_owned(),
            description: concat!(
                "Search the live web with Google. Use it when the user asks ",
                "about current events, facts you are unsure of, or anything ",
                "newer than your training. Results are titles, links and ",
                "snippets, best matches first — cite the link when you lean ",
                "on a result, and use web_fetch to read a promising one.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What to search for, like a Google query."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (default 5, max 10)."
                    }
                },
                "required": ["query"]
            }),
        });
    }
    // MAIL. Ground is an identity to sign with plus the local database — both
    // present on any real turn, absent only in tests and in the seconds before
    // a companion is resolved.
    if context.companion_id.is_some() && context.database_path.is_some() {
        tools.push(ToolDeclaration {
            name: LIST_AGENTS.to_owned(),
            description: concat!(
                "List the other agents you can write to, with their ids. ",
                "Call this before send_message when you do not already know ",
                "the recipient's id — an id is the only way to address ",
                "someone, and names are not unique.",
            )
            .to_owned(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        });
        tools.push(ToolDeclaration {
            name: SEND_MESSAGE.to_owned(),
            description: concat!(
                "Send a message to another agent. It waits in their inbox ",
                "until they read it — they are not interrupted, and you will ",
                "not get a reply in this turn. Your own id is attached ",
                "automatically; you cannot send as anyone else. Say who you ",
                "are and what you want in the message itself, because the ",
                "recipient may read it in a conversation you are not part of.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "to_agent_id": {
                        "type": "string",
                        "description": "The recipient's agent id, from list_agents."
                    },
                    "body": {
                        "type": "string",
                        "description": "What you want to say. Plain text."
                    }
                },
                "required": ["to_agent_id", "body"]
            }),
        });
        tools.push(ToolDeclaration {
            name: READ_MESSAGES.to_owned(),
            description: concat!(
                "Read your inbox — messages other agents have sent you. ",
                "Defaults to unread only. Reading does NOT mark anything ",
                "read; call mark_message_read once you have actually dealt ",
                "with a message.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "unread_only": {
                        "type": "boolean",
                        "description": "Default true. False returns read mail too."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max messages to return (default 20, max 100)."
                    }
                }
            }),
        });
        tools.push(ToolDeclaration {
            name: MARK_MESSAGE_READ.to_owned(),
            description: concat!(
                "Mark one message in your inbox as read, once you have acted ",
                "on it or decided it needs nothing. Only your own mail — you ",
                "cannot clear anyone else's.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "message_id": {
                        "type": "string",
                        "description": "The id of the message, from read_messages."
                    }
                },
                "required": ["message_id"]
            }),
        });

        // CALLS. Same ground as mail, and the same identity signs them.
        //
        // A letter waits in a drawer; a CALL is an exchange with a shape — it
        // belongs to the conversation it was opened from, it holds both sides'
        // turns in order, and it runs out. The description tells the model the
        // limits UP FRONT rather than only at the refusal, so it can decide
        // whether a question is worth a call before spending one.
        tools.push(ToolDeclaration {
            name: OPEN_CALL.to_owned(),
            description: concat!(
                "Start a call with another agent and say your first line. Use ",
                "this instead of send_message when you expect a back-and-forth ",
                "rather than a note — a call keeps both sides' turns together ",
                "and belongs to this conversation, so the user can see what ",
                "was said on their behalf. ",
                "You may open only 5 calls a day, and each call holds only 5 ",
                "messages from both sides together, so open one for something ",
                "worth the exchange. The other agent will not answer inside ",
                "this turn.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "to_agent_id": {
                        "type": "string",
                        "description": concat!(
                            "The agent's ID from list_agents — a uuid, NOT a ",
                            "name. Call list_agents first if you do not have ",
                            "the exact id in front of you; a name is refused."
                        )
                    },
                    "body": {
                        "type": "string",
                        "description": concat!(
                            "Your opening line. Say who you are and what you ",
                            "want — they may read it with no other context."
                        )
                    }
                },
                "required": ["to_agent_id", "body"]
            }),
        });
        tools.push(ToolDeclaration {
            name: SEND_IN_CALL.to_owned(),
            description: concat!(
                "Say something into a call that is already open. This does NOT ",
                "cost one of your 5 daily calls — replying inside a call is ",
                "free, and the call's own 5-message limit still applies. The ",
                "call closes itself on its last message.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "call_id": {
                        "type": "string",
                        "description": "The call, from open_call or list_calls."
                    },
                    "body": {
                        "type": "string",
                        "description": "What you want to say. Plain text."
                    }
                },
                "required": ["call_id", "body"]
            }),
        });
        tools.push(ToolDeclaration {
            name: READ_CALL.to_owned(),
            description: concat!(
                "Read everything said in one call, oldest first. Only calls ",
                "you are part of.",
            )
            .to_owned(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "call_id": {
                        "type": "string",
                        "description": "The call, from list_calls or open_call."
                    }
                },
                "required": ["call_id"]
            }),
        });
        tools.push(ToolDeclaration {
            name: LIST_CALLS.to_owned(),
            description: concat!(
                "List the open calls you are part of, including ones another ",
                "agent opened with you, and how many of today's calls you have ",
                "left. Check this when you want to know whether someone is ",
                "waiting on you.",
            )
            .to_owned(),
            parameters: serde_json::json!({ "type": "object", "properties": {} }),
        });
    }
    // Fetch's ground is the network itself, so it always rides.
    tools.push(ToolDeclaration {
        name: WEB_FETCH.to_owned(),
        description: concat!(
            "Read one web page: fetches the URL and returns its readable ",
            "text (title, headings, prose — no markup). Use it on a link ",
            "the user gives you or a promising web_search result. Some ",
            "pages render only in a browser and will come back empty — ",
            "say so and lean on the search snippets instead.",
        )
        .to_owned(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The http(s) URL to read."
                }
            },
            "required": ["url"]
        }),
    });
    tools
}

/// Execute one call. `Err` is a tool-level failure the model should read and
/// recover from — the chat loop folds it back as the tool's result, it never
/// kills the stream.
/// Where this turn's memory tools write. Falls back to the organ when a target
/// was never resolved — the shape every install but this one has — so an older
/// caller that only set `memory_agent_id` still reaches the same place it did.
fn memory_target(context: &ToolContext) -> Result<memory::MemoryTarget, String> {
    if let Some(target) = context.memory_target.clone() {
        return Ok(target);
    }
    context
        .memory_agent_id
        .as_deref()
        .map(|agent_id| memory::MemoryTarget::Organ { agent_id: agent_id.to_owned() })
        .ok_or_else(|| "memory is not connected for this conversation".to_owned())
}

pub(crate) async fn execute(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    match call.name.as_str() {
        RECALL_MEMORY => {
            let target = memory_target(context)?;
            let name = parse_name_argument(&call.arguments)?;
            let memory = memory::fetch_memory(&target, &name).await?;
            Ok(render_memory(&memory))
        }
        CARVE_MEMORY => {
            let target = memory_target(context)?;
            let payload = parse_carve_arguments(&call.arguments)?;
            let result = memory::write_memory(&target, &payload).await?;
            Ok(render_carve_outcome(&payload, &result))
        }
        SEARCH_CONVERSATIONS => {
            let path = context
                .database_path
                .clone()
                .ok_or_else(|| "the conversation archive is not available".to_owned())?;
            let (query, limit) = parse_search_arguments(&call.arguments)?;
            let fts_query = fts_match_expression(&query)
                .ok_or_else(|| "give at least one word to search for".to_owned())?;
            let exclude = context.conversation_id.clone();
            let hits = tauri::async_runtime::spawn_blocking(move || {
                let repository = ChatRepository::open(&path).map_err(String::from)?;
                repository
                    .search_messages(&fts_query, exclude.as_deref(), limit)
                    .map_err(String::from)
            })
            .await
            .map_err(|error| format!("the archive search task failed: {error}"))??;
            Ok(render_archive_hits(&query, &hits))
        }
        WEB_SEARCH => {
            let api_key = context
                .serpapi_api_key
                .clone()
                .ok_or_else(|| "web search is not available on this machine".to_owned())?;
            let (query, limit) = parse_web_search_arguments(&call.arguments)?;
            let harvest = web::search(&query, limit, &api_key).await?;
            Ok(render_web_search(&query, &harvest))
        }
        WEB_FETCH => {
            let url = parse_url_argument(&call.arguments)?;
            let page = web::fetch(&url).await?;
            Ok(render_web_page(&url, &page))
        }
        LIST_AGENTS | SEND_MESSAGE | READ_MESSAGES | MARK_MESSAGE_READ => {
            // Every mail tool needs the same two things, so they are resolved
            // once here rather than four times below.
            let me = context
                .companion_id
                .clone()
                .ok_or_else(|| "no companion identity for this turn".to_owned())?;
            let path = context
                .database_path
                .clone()
                .ok_or_else(|| "the local database is not available".to_owned())?;
            let name = call.name.clone();
            let arguments = call.arguments.clone();
            // Blocking sqlite work off the async runtime, same as the file
            // tools — the mail store opens its own connection.
            tauri::async_runtime::spawn_blocking(move || {
                execute_mail(&name, &arguments, &me, &path)
            })
            .await
            .map_err(|error| format!("the mail task failed: {error}"))?
        }
        OPEN_CALL | SEND_IN_CALL | READ_CALL | LIST_CALLS => {
            let me = context
                .companion_id
                .clone()
                .ok_or_else(|| "no companion identity for this turn".to_owned())?;
            let path = context
                .database_path
                .clone()
                .ok_or_else(|| "the local database is not available".to_owned())?;
            // ⚑ THE ROOT CONVERSATION COMES FROM THE TURN, NOT THE MODEL.
            // Same law as the sender's id: there is no parameter a companion
            // could put a different conversation in, so where a call came from
            // is a fact about the turn rather than a claim the model makes.
            let root = context.conversation_id.clone();
            let name = call.name.clone();
            let arguments = call.arguments.clone();
            tauri::async_runtime::spawn_blocking(move || {
                execute_call(&name, &arguments, &me, root.as_deref(), &path)
            })
            .await
            .map_err(|error| format!("the call task failed: {error}"))?
        }
        name if files::FILE_TOOL_NAMES.contains(&name) => {
            let root = context
                .workspace_dir
                .clone()
                .ok_or_else(|| "no workspace folder is connected for this companion".to_owned())?;
            let name = call.name.clone();
            let arguments = call.arguments.clone();
            tauri::async_runtime::spawn_blocking(move || files::execute(&name, &arguments, &root))
                .await
                .map_err(|error| format!("the file task failed: {error}"))?
        }
        other => Err(format!("unknown tool \"{other}\"")),
    }
}

/// The four mail tools, synchronous because sqlite is.
///
/// `me` is the speaking companion's id, taken from the resolved turn — never
/// from `arguments`. That is the whole reason a companion cannot forge a
/// sender: there is no parameter it could put a different id in.
fn execute_mail(name: &str, arguments: &str, me: &str, path: &Path) -> Result<String, String> {
    let mail = AgentMailRepository::open(path).map_err(|error| error.to_string())?;

    match name {
        LIST_AGENTS => {
            let roster = CompanionRepository::open(path).map_err(|error| error.to_string())?;
            let companions = roster.list().map_err(|error| error.to_string())?;
            Ok(render_agents(&companions, me))
        }
        SEND_MESSAGE => {
            let (to, body) = parse_send_arguments(arguments)?;
            if to == me {
                return Err("that is your own id — you cannot write to yourself".to_owned());
            }
            let sent = mail
                .send(SendAgentMessage {
                    from_agent_id: me,
                    to_agent_id: &to,
                    from_user_id: None,
                    to_user_id: None,
                    project_id: None,
                    body: &body,
                })
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "[sent to {} · id {} · it waits in their inbox until they read it]",
                sent.to_agent_id, sent.id
            ))
        }
        READ_MESSAGES => {
            let (unread_only, limit) = parse_inbox_arguments(arguments)?;
            let mut messages = mail.inbox(me, unread_only).map_err(|error| error.to_string())?;
            messages.truncate(limit as usize);
            Ok(render_inbox(&messages, unread_only))
        }
        MARK_MESSAGE_READ => {
            let id = parse_message_id_argument(arguments)?;
            if mail.mark_read(me, &id).map_err(|error| error.to_string())? {
                Ok(format!("[{id} marked read]"))
            } else {
                // One message for both causes on purpose: "already read" and
                // "not yours" must not be distinguishable, or the tool becomes
                // an oracle for whether someone else's message id exists.
                Ok(format!(
                    "[nothing to do — {id} is not an unread message in your inbox]"
                ))
            }
        }
        other => Err(format!("unknown mail tool \"{other}\"")),
    }
}

/// The call tools. `me` signs every turn and `root` is the conversation this
/// turn belongs to — both come from the turn, neither from the model.
fn execute_call(
    name: &str,
    arguments: &str,
    me: &str,
    root: Option<&str>,
    path: &Path,
) -> Result<String, String> {
    let calls = RavenCallRepository::open(path).map_err(|error| error.to_string())?;

    match name {
        OPEN_CALL => {
            let (to, body) = parse_send_arguments(arguments)?;
            if to == me {
                return Err("that is your own id — you cannot call yourself".to_owned());
            }

            // ⚑ THE RECIPIENT MUST BE SOMEONE WHO CAN ACTUALLY PICK UP.
            // `send_message` deliberately does NOT check this — a letter may be
            // addressed to an agent on another machine and wait. A CALL cannot:
            // the only thing that can answer one is a companion this process is
            // able to wake, so an unresolvable recipient is a call that will
            // ring forever while still spending one of today's five.
            //
            // Found the hard way (s502): asked to ring Hugin, the model passed
            // the NAME "hugin" instead of the id, and made a call nobody could
            // answer. The description already said "from list_agents"; saying
            // it was not enough, which is the whole lesson of the manifest.
            let roster = CompanionRepository::open(path).map_err(|error| error.to_string())?;
            let companions = roster.list().map_err(|error| error.to_string())?;
            if !companions.iter().any(|companion| companion.id == to) {
                // The roster rides the refusal so the model can correct itself
                // in this same turn instead of calling list_agents and retrying.
                return Err(format!(
                    "\"{to}\" is not an agent id, so nobody could ever answer that call — \
                     names are not addresses. Use one of these ids:\n{}",
                    render_agents(&companions, me)
                ));
            }

            // ⚑ OPENING AND SPEAKING ARE ONE ACT, DELIBERATELY. A tool that
            // only opened an empty call would let a confused model spend the
            // whole day's allowance on five silent rooms, and would cost a
            // tool round to say anything. There is no way to open a call
            // without saying something in it.
            let call = calls
                .open_call(me, root)
                .map_err(|error| error.to_string())?;
            calls
                .append_message(&call.id, me, &to, &body)
                .map_err(|error| error.to_string())?;

            let left = calls
                .calls_remaining_today(me)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "[call opened with {to} · id {} · your line was sent]\n\
                 [{} of today's calls left · {} more messages fit in this call · \
                 they will not answer inside this turn]",
                call.id,
                left,
                raven_calls::MAX_MESSAGES_PER_CALL - 1,
            ))
        }
        SEND_IN_CALL => {
            let (call_id, body) = parse_call_message_arguments(arguments)?;

            // Scoped BEFORE the write: without this, knowing a uuid would be
            // enough to speak into someone else's call.
            let turns = calls
                .messages_visible_to(&call_id, me, CALL_READ_LIMIT)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| {
                    "there is no open call with that id that you are part of".to_owned()
                })?;
            let to = other_end(&turns, me).ok_or_else(|| {
                "that call has nobody else in it yet — open_call starts one".to_owned()
            })?;

            calls
                .append_message(&call_id, me, &to, &body)
                .map_err(|error| error.to_string())?;

            let remaining = raven_calls::MAX_MESSAGES_PER_CALL - (turns.len() as i64 + 1);
            Ok(if remaining <= 0 {
                "[sent · that was the call's last message, so it is now closed]".to_owned()
            } else {
                format!("[sent to {to} · {remaining} more messages fit in this call]")
            })
        }
        READ_CALL => {
            let call_id = parse_call_id_argument(arguments)?;
            match calls
                .messages_visible_to(&call_id, me, CALL_READ_LIMIT)
                .map_err(|error| error.to_string())?
            {
                // One answer for "not yours" and "not there", the same reason
                // mark_message_read has one: otherwise the tool is an oracle
                // for whether someone else's call exists.
                None => Ok(format!(
                    "[nothing to read — {call_id} is not a call you are part of]"
                )),
                Some(turns) => Ok(render_call(&call_id, &turns)),
            }
        }
        LIST_CALLS => {
            let open = calls
                .open_calls_for_agent(me)
                .map_err(|error| error.to_string())?;
            let left = calls
                .calls_remaining_today(me)
                .map_err(|error| error.to_string())?;
            Ok(render_calls(&open, left))
        }
        other => Err(format!("unknown call tool \"{other}\"")),
    }
}

/// Who the other end of a call is, read from its turns rather than stored.
/// A call has exactly two participants, so the first id that is not mine is
/// the answer — and taking it from the record means a reply cannot be
/// redirected to a third party by argument.
fn other_end(turns: &[raven_calls::RavenCallMessage], me: &str) -> Option<String> {
    turns.iter().find_map(|turn| {
        if turn.from_agent_id != me {
            Some(turn.from_agent_id.clone())
        } else if turn.to_agent_id != me {
            Some(turn.to_agent_id.clone())
        } else {
            None
        }
    })
}

fn render_call(call_id: &str, turns: &[raven_calls::RavenCallMessage]) -> String {
    if turns.is_empty() {
        return format!("[call {call_id} has nothing in it yet]");
    }
    let mut out = format!("Call {call_id} — {} messages:\n", turns.len());
    for turn in turns {
        out.push_str(&format!("\n── {}\n{}\n", turn.from_agent_id, turn.body));
    }
    let left = raven_calls::MAX_MESSAGES_PER_CALL - turns.len() as i64;
    out.push_str(&if left > 0 {
        format!("\n[{left} more messages fit · reply with send_in_call]")
    } else {
        "\n[this call is full and closed]".to_owned()
    });
    out
}

fn render_calls(open: &[raven_calls::RavenCall], calls_left: i64) -> String {
    let mut out = if open.is_empty() {
        "[no open calls]".to_owned()
    } else {
        let mut listed = format!("{} open call(s):\n", open.len());
        for call in open {
            listed.push_str(&format!(
                "  · id {} · opened by {} · {} of {} messages used\n",
                call.id,
                call.initiator_agent_id,
                call.message_count,
                raven_calls::MAX_MESSAGES_PER_CALL,
            ));
        }
        listed
    };
    out.push_str(&format!(
        "\n[{calls_left} of today's {} calls left · the allowance resets at midnight]",
        raven_calls::MAX_CALLS_PER_DAY
    ));
    out
}

fn parse_call_message_arguments(arguments: &str) -> Result<(String, String), String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let call_id = parsed
        .get("call_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"call_id\" is required — call list_calls for ids".to_owned())?;
    let body = parsed
        .get("body")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"body\" argument is required".to_owned())?;
    Ok((call_id, body))
}

fn parse_call_id_argument(arguments: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    parsed
        .get("call_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"call_id\" argument is required".to_owned())
}

fn parse_send_arguments(arguments: &str) -> Result<(String, String), String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let to = parsed
        .get("to_agent_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            "a non-empty \"to_agent_id\" is required — call list_agents for ids".to_owned()
        })?;
    let body = parsed
        .get("body")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"body\" argument is required".to_owned())?;
    Ok((to, body))
}

fn parse_inbox_arguments(arguments: &str) -> Result<(bool, u32), String> {
    // An absent or empty argument object is normal here — every parameter has
    // a default, so `read_messages` with no arguments must succeed.
    let parsed: serde_json::Value =
        serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
    let unread_only = parsed
        .get("unread_only")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let limit = parsed
        .get("limit")
        .and_then(|value| value.as_u64())
        .map(|value| value.clamp(1, u64::from(INBOX_MAX_LIMIT)) as u32)
        .unwrap_or(INBOX_DEFAULT_LIMIT);
    Ok((unread_only, limit))
}

fn parse_message_id_argument(arguments: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    parsed
        .get("message_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"message_id\" argument is required".to_owned())
}

fn render_agents(companions: &[Companion], me: &str) -> String {
    let others: Vec<&Companion> = companions
        .iter()
        .filter(|companion| companion.id != me)
        .collect();
    if others.is_empty() {
        return "[no other agents on this machine yet — you are the only one]".to_owned();
    }

    let mut out = String::from("Agents you can write to:\n");
    for companion in others {
        let name = companion.name.as_deref().unwrap_or("(unnamed)");
        out.push_str(&format!("  · {name} — id {}\n", companion.id));
    }
    out.push_str("\n[address send_message with the id, not the name]");
    out
}

fn render_inbox(messages: &[super::agent_mail::AgentMessage], unread_only: bool) -> String {
    if messages.is_empty() {
        return if unread_only {
            "[no unread messages]".to_owned()
        } else {
            "[your inbox is empty]".to_owned()
        };
    }

    let mut out = format!(
        "{} message{} in your inbox:\n",
        messages.len(),
        if messages.len() == 1 { "" } else { "s" }
    );
    for message in messages {
        out.push_str(&format!(
            "\n── from {} · id {}{}\n{}\n",
            message.from_agent_id,
            message.id,
            if message.read_at.is_some() { " · read" } else { "" },
            message.body,
        ));
    }
    out.push_str("\n[reply with send_message; mark_message_read once you have dealt with one]");
    out
}

fn parse_search_arguments(arguments: &str) -> Result<(String, u32), String> {
    parse_query_and_limit(arguments, SEARCH_DEFAULT_LIMIT, SEARCH_MAX_LIMIT)
}

fn parse_web_search_arguments(arguments: &str) -> Result<(String, u32), String> {
    parse_query_and_limit(arguments, WEB_SEARCH_DEFAULT_LIMIT, WEB_SEARCH_MAX_LIMIT)
}

fn parse_query_and_limit(
    arguments: &str,
    default_limit: u32,
    max_limit: u32,
) -> Result<(String, u32), String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let query = parsed
        .get("query")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"query\" argument is required".to_owned())?;
    let limit = parsed
        .get("limit")
        .and_then(|value| value.as_u64())
        .map(|limit| (limit as u32).clamp(1, max_limit))
        .unwrap_or(default_limit);
    Ok((query, limit))
}

/// FTS5 MATCH syntax gives operators to bare input ("don't" parses as a
/// column filter, "AND" as boolean) — so every term is passed as a quoted
/// string instead: split on whitespace, double any inner quotes, join with
/// implicit AND. None = nothing searchable survived.
fn fts_match_expression(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return None;
    }
    Some(terms.join(" "))
}

/// Hits rendered for the model: one block per WHOLE message — who said it,
/// when, in which conversation, then the full text. Size is governed by
/// dropping trailing messages once the char budget is spent, never by
/// cutting one open; the best match is always included whole.
fn render_archive_hits(query: &str, hits: &[ArchiveHit]) -> String {
    if hits.is_empty() {
        return format!("nothing in your past conversations matches \"{query}\"");
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut spent = 0usize;
    for hit in hits {
        let who = match hit.role.as_str() {
            "user" => "the user said",
            _ => "you said",
        };
        let block = format!(
            "[{} · {} · {}]\n{}",
            hit.day, hit.conversation_title, who, hit.content
        );
        if !blocks.is_empty() && spent + block.len() > SEARCH_RENDER_BUDGET_CHARS {
            break;
        }
        spent += block.len();
        blocks.push(block);
    }
    let withheld = hits.len() - blocks.len();
    let mut rendered = format!(
        "{} message(s) from your past conversations, best match first:\n\n{}",
        blocks.len(),
        blocks.join("\n\n")
    );
    if withheld > 0 {
        rendered.push_str(&format!(
            "\n\n({withheld} more matching message(s) withheld to stay readable — narrow the query to reach them)"
        ));
    }
    rendered
}

/// A web harvest rendered for the model: Google's direct answer first when
/// one came back, then one block per result — title, link, snippet.
fn render_web_search(query: &str, harvest: &web::WebSearch) -> String {
    if harvest.answer.is_none() && harvest.hits.is_empty() {
        return format!("the web search for \"{query}\" returned nothing");
    }
    let mut sections: Vec<String> = Vec::new();
    if let Some(answer) = &harvest.answer {
        sections.push(format!("⚡ {answer}"));
    }
    for hit in &harvest.hits {
        let mut block = format!("{}\n{}", hit.title, hit.link);
        if !hit.snippet.is_empty() {
            block.push('\n');
            block.push_str(&hit.snippet);
        }
        sections.push(block);
    }
    format!(
        "web results for \"{query}\", best match first:\n\n{}",
        sections.join("\n\n")
    )
}

fn parse_url_argument(arguments: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    parsed
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"url\" argument is required".to_owned())
}

/// A fetched page rendered for the model: title line, then the extracted
/// text, truncated at the budget with an honest marker. An empty extraction
/// names the JS-shell cause instead of pretending the page said nothing.
fn render_web_page(url: &str, page: &web::WebPage) -> String {
    if page.text.is_empty() {
        return format!(
            "\"{url}\" renders only in a browser (a JS shell) — its readable \
             text could not be extracted; lean on search snippets instead"
        );
    }
    let mut rendered = match &page.title {
        Some(title) if !title.is_empty() => format!("{title} — {url}\n\n"),
        _ => format!("{url}\n\n"),
    };
    if page.text.len() > WEB_FETCH_RENDER_BUDGET_CHARS {
        let mut cut = WEB_FETCH_RENDER_BUDGET_CHARS;
        while !page.text.is_char_boundary(cut) {
            cut -= 1;
        }
        rendered.push_str(&page.text[..cut]);
        rendered.push_str("\n\n(truncated — the page continues past this point)");
    } else {
        rendered.push_str(&page.text);
    }
    rendered
}

fn parse_name_argument(arguments: &str) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    parsed
        .get("name")
        .and_then(|name| name.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "a non-empty \"name\" argument is required".to_owned())
}

/// Reshape the model's carve arguments into the organ's MemoryWriteRequest.
/// The model's `type` becomes the organ's `mem_type`; project_tag is pinned
/// here (the model never chooses the shelf). Validation beyond presence stays
/// the organ's job — its errors fold back as readable tool results.
fn parse_carve_arguments(arguments: &str) -> Result<serde_json::Value, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let text_field = |key: &str| -> Result<String, String> {
        parsed
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("a non-empty \"{key}\" argument is required"))
    };
    let mut payload = serde_json::json!({
        "name": text_field("name")?,
        "description": text_field("description")?,
        "body": text_field("body")?,
        "project_tag": "companion",
    });
    if let Some(mem_type) = parsed.get("type").and_then(|value| value.as_str()) {
        payload["mem_type"] = serde_json::json!(mem_type);
    }
    if let Some(importance) = parsed.get("importance").and_then(|value| value.as_f64()) {
        payload["importance"] = serde_json::json!(importance.clamp(0.0, 1.0));
    }
    Ok(payload)
}

/// What the model reads back after carving — confirms the name it can later
/// recall by, and says whether the carve created or overwrote.
fn render_carve_outcome(payload: &serde_json::Value, result: &serde_json::Value) -> String {
    let name = payload["name"].as_str().unwrap_or("the memory");
    let verb = match result.get("created").and_then(|value| value.as_bool()) {
        Some(false) => "updated",
        _ => "carved",
    };
    format!("{verb} [{name}] — it will be there next time")
}

/// The organ's MemoryOut, rendered in the same row shape the ambient
/// injection uses — one format for memory however it arrives.
fn render_memory(memory: &serde_json::Value) -> String {
    let field = |key: &str| memory.get(key).and_then(|value| value.as_str()).unwrap_or("");
    let name = field("name");
    if name.is_empty() {
        // Schema drifted — hand the model the raw JSON rather than nothing.
        return memory.to_string();
    }
    let created = field("created_at").chars().take(10).collect::<String>();
    let mut header = format!("[{name}] ({}", field("mem_type"));
    if !created.is_empty() {
        header.push_str(&format!(" · carved {created}"));
    }
    header.push_str(&format!(") — {}", field("description")));
    format!("{header}\n{}", field("body"))
}

#[cfg(test)]
mod tests {
    use super::{
        declarations, fts_match_expression, parse_carve_arguments, parse_name_argument,
        parse_search_arguments, parse_url_argument, parse_web_search_arguments,
        render_archive_hits, render_carve_outcome, render_memory, render_web_page,
        render_web_search, ToolContext,
    };
    use super::{execute_mail, LIST_AGENTS, MARK_MESSAGE_READ, READ_MESSAGES, SEND_MESSAGE};
    use super::{execute_call, LIST_CALLS, OPEN_CALL, READ_CALL, SEND_IN_CALL};
    use crate::chat::repository::ArchiveHit;
    use crate::raven_calls::{self, RavenCallRepository};
    use crate::web;

    /// A real database, migrated, with the built-in companion in it.
    fn mail_fixture(tag: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "companion-mailtool-test-{tag}-{}.db",
            uuid::Uuid::new_v4()
        ));
        crate::database::initialise(&path).expect("test database should initialise");
        path
    }

    /// A committed conversation for a call to hang from.
    ///
    /// ⚑ NOT A TEST CONVENIENCE — the FK from `raven_calls` is real, and an
    /// invented id is refused outright. That is the constraint working: in a
    /// live turn the conversation is committed by `submit` before any tool
    /// runs, so a call's provenance can never point at a thread that is not
    /// there. A test that faked the id would have been testing a world we
    /// deliberately made impossible.
    fn conversation(path: &std::path::Path, id: &str) -> String {
        let now = crate::credentials::unix_timestamp_ms().expect("a clock");
        rusqlite::Connection::open(path)
            .expect("database should open")
            .execute(
                "INSERT INTO conversations (id, title, created_at, updated_at)
                 VALUES (?1, 'a thread', ?2, ?2)",
                rusqlite::params![id, now],
            )
            .expect("the conversation should insert");
        id.to_owned()
    }

    /// A real second companion to call.
    ///
    /// ⚑ NOT A FIXTURE CONVENIENCE. Every call test used to invent a uuid for
    /// the recipient, which meant they all passed while the live app was making
    /// calls nobody could answer — the tests agreed with each other about a
    /// world where a recipient need not exist. A recipient has to be somebody.
    fn companion(path: &std::path::Path, name: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::credentials::unix_timestamp_ms().expect("a clock");
        rusqlite::Connection::open(path)
            .expect("database should open")
            .execute(
                "INSERT INTO companions (
                    id, name, memory_agent_name, is_built_in,
                    model_preference_mode, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 0, 'inherit', ?4, ?4)",
                rusqlite::params![id, name, format!("agent-{id}"), now],
            )
            .expect("the companion should insert");
        id
    }

    fn built_in(path: &std::path::Path) -> String {
        rusqlite::Connection::open(path)
            .expect("database should open")
            .query_row("SELECT id FROM companions WHERE is_built_in = 1", [], |row| {
                row.get(0)
            })
            .expect("the built-in companion should exist")
    }

    #[test]
    fn an_empty_inbox_says_so_rather_than_failing() {
        let path = mail_fixture("empty");
        let me = built_in(&path);

        // No arguments at all — every parameter has a default, so the bare
        // call a model is most likely to make must work.
        let unread = execute_mail(READ_MESSAGES, "{}", &me, &path).expect("reading should succeed");
        assert_eq!(unread, "[no unread messages]");

        let all = execute_mail(READ_MESSAGES, r#"{"unread_only":false}"#, &me, &path)
            .expect("reading should succeed");
        assert_eq!(all, "[your inbox is empty]");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn list_agents_shows_the_others_and_never_yourself() {
        let path = mail_fixture("roster");
        let me = built_in(&path);

        // A lone companion has nobody to write to, and the tool says that
        // plainly instead of returning an empty list the model must interpret.
        let alone = execute_mail(LIST_AGENTS, "{}", &me, &path).expect("listing should succeed");
        assert!(alone.contains("no other agents"), "got: {alone}");
        assert!(!alone.contains(&me), "you are never in your own roster");

        // Now with company: a named one and an unnamed one, since a companion
        // may have no name and the roster still has to be addressable.
        let huginn = uuid::Uuid::new_v4().to_string();
        let nameless = uuid::Uuid::new_v4().to_string();
        let connection = rusqlite::Connection::open(&path).expect("database should open");
        for (id, name) in [(&huginn, Some("Huginn")), (&nameless, None)] {
            connection
                .execute(
                    "INSERT INTO companions (id, name, memory_agent_name, is_built_in,
                        model_preference_mode, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 0, 'inherit', 0, 0)",
                    rusqlite::params![id, name, format!("companion-{id}")],
                )
                .expect("a companion should insert");
        }
        drop(connection);

        let roster = execute_mail(LIST_AGENTS, "{}", &me, &path).expect("listing should succeed");
        assert!(roster.contains("Huginn"), "got: {roster}");
        assert!(roster.contains(&huginn), "the id is what send_message needs");
        assert!(roster.contains("(unnamed)"), "an unnamed companion is still listed");
        assert!(roster.contains(&nameless), "and still addressable");
        assert!(!roster.contains(&me), "you are never in your own roster");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_letter_round_trips_between_two_agents() {
        let path = mail_fixture("round-trip");
        let rook = built_in(&path);
        let huginn = uuid::Uuid::new_v4().to_string();

        let sent = execute_mail(
            SEND_MESSAGE,
            &serde_json::json!({ "to_agent_id": &huginn, "body": "Shall we settle the name?" })
                .to_string(),
            &rook,
            &path,
        )
        .expect("sending should succeed");
        assert!(sent.contains("waits in their inbox"), "got: {sent}");

        // The recipient sees it; the sender's own inbox stays empty.
        let inbox = execute_mail(READ_MESSAGES, "{}", &huginn, &path).expect("inbox should read");
        assert!(inbox.contains("Shall we settle the name?"), "got: {inbox}");
        assert!(inbox.contains(&rook), "the letter names its sender");
        assert_eq!(
            execute_mail(READ_MESSAGES, "{}", &rook, &path).unwrap(),
            "[no unread messages]"
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_companion_cannot_forge_a_sender_or_write_to_itself() {
        let path = mail_fixture("forge");
        let rook = built_in(&path);
        let huginn = uuid::Uuid::new_v4().to_string();

        // There is no `from` parameter, so an attempt to supply one is simply
        // an unknown key — the sender stays whoever the turn belongs to.
        execute_mail(
            SEND_MESSAGE,
            &serde_json::json!({
                "to_agent_id": &huginn,
                "from_agent_id": "somebody-else",
                "body": "Not who you think."
            })
            .to_string(),
            &rook,
            &path,
        )
        .expect("sending should succeed");
        let inbox = execute_mail(READ_MESSAGES, "{}", &huginn, &path).unwrap();
        assert!(inbox.contains(&rook), "the real sender is recorded");
        assert!(!inbox.contains("somebody-else"), "the forged sender is ignored");

        let error = execute_mail(
            SEND_MESSAGE,
            &serde_json::json!({ "to_agent_id": &rook, "body": "hello me" }).to_string(),
            &rook,
            &path,
        )
        .expect_err("writing to yourself should be refused");
        assert!(error.contains("your own id"), "got: {error}");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn marking_read_is_scoped_and_tells_a_stranger_nothing() {
        let path = mail_fixture("mark");
        let rook = built_in(&path);
        let huginn = uuid::Uuid::new_v4().to_string();
        let magpie = uuid::Uuid::new_v4().to_string();

        execute_mail(
            SEND_MESSAGE,
            &serde_json::json!({ "to_agent_id": &huginn, "body": "Read me." }).to_string(),
            &rook,
            &path,
        )
        .unwrap();
        let inbox = execute_mail(READ_MESSAGES, "{}", &huginn, &path).unwrap();
        let id = inbox
            .split("id ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("the inbox should print an id")
            .to_owned();

        // A stranger gets the SAME sentence as a no-op, so the tool cannot be
        // used to discover whether someone else's message id is real.
        let stranger = execute_mail(
            MARK_MESSAGE_READ,
            &serde_json::json!({ "message_id": &id }).to_string(),
            &magpie,
            &path,
        )
        .unwrap();
        let invented = execute_mail(
            MARK_MESSAGE_READ,
            &serde_json::json!({ "message_id": "no-such-message" }).to_string(),
            &magpie,
            &path,
        )
        .unwrap();
        assert!(stranger.contains("nothing to do"), "got: {stranger}");
        assert_eq!(
            stranger.replace(&id, "X"),
            invented.replace("no-such-message", "X"),
            "a real id and an invented one must be indistinguishable to a stranger"
        );

        // Still unread for its actual owner, who can clear it.
        assert!(execute_mail(READ_MESSAGES, "{}", &huginn, &path)
            .unwrap()
            .contains("Read me."));
        let owner = execute_mail(
            MARK_MESSAGE_READ,
            &serde_json::json!({ "message_id": &id }).to_string(),
            &huginn,
            &path,
        )
        .unwrap();
        assert!(owner.contains("marked read"), "got: {owner}");
        assert_eq!(
            execute_mail(READ_MESSAGES, "{}", &huginn, &path).unwrap(),
            "[no unread messages]"
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_mail_tools_ride_only_when_there_is_an_identity_to_sign_with() {
        let anonymous = declarations(&ToolContext {
            database_path: Some("companion.db".into()),
            ..Default::default()
        });
        assert!(
            !anonymous.iter().any(|tool| tool.name == SEND_MESSAGE),
            "no companion id means no mail tools at all"
        );

        let signed = declarations(&ToolContext {
            database_path: Some("companion.db".into()),
            companion_id: Some("companion-1".to_owned()),
            ..Default::default()
        });
        for expected in [
            LIST_AGENTS,
            SEND_MESSAGE,
            READ_MESSAGES,
            MARK_MESSAGE_READ,
            OPEN_CALL,
            SEND_IN_CALL,
            READ_CALL,
            LIST_CALLS,
        ] {
            assert!(
                signed.iter().any(|tool| tool.name == expected),
                "{expected} should be declared"
            );
        }
    }

    #[test]
    fn a_call_round_trips_and_a_reply_costs_the_replier_nothing() {
        let path = mail_fixture("call-round-trip");
        let rook = built_in(&path);
        let hugin = companion(&path, "Hugin");

        let root = conversation(&path, "conv-1");

        let opened = execute_call(
            OPEN_CALL,
            &serde_json::json!({ "to_agent_id": &hugin, "body": "Are you awake?" }).to_string(),
            &rook,
            Some(&root),
            &path,
        )
        .expect("opening a call should succeed");
        assert!(opened.contains("call opened"), "got: {opened}");
        assert!(opened.contains("4 of today's calls left"), "got: {opened}");

        // The recipient finds the call without being told its id.
        let listed = execute_call(LIST_CALLS, "{}", &hugin, None, &path).expect("list should work");
        assert!(listed.contains("1 open call"), "got: {listed}");
        assert!(
            listed.contains("5 of today's 5 calls left"),
            "being called costs the recipient nothing: {listed}"
        );

        let call_id = listed
            .split("id ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("the listing names the call id")
            .to_owned();

        let replied = execute_call(
            SEND_IN_CALL,
            &serde_json::json!({ "call_id": &call_id, "body": "I am." }).to_string(),
            &hugin,
            None,
            &path,
        )
        .expect("replying should succeed");
        assert!(replied.contains(&rook), "the reply is addressed back: {replied}");
        assert!(replied.contains("3 more messages"), "got: {replied}");

        let read = execute_call(
            READ_CALL,
            &serde_json::json!({ "call_id": &call_id }).to_string(),
            &rook,
            None,
            &path,
        )
        .expect("reading should succeed");
        assert!(read.contains("Are you awake?"), "got: {read}");
        assert!(read.contains("I am."), "got: {read}");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn calling_a_name_instead_of_an_id_is_refused_with_the_roster() {
        let path = mail_fixture("call-by-name");
        let rook = built_in(&path);

        // ⚑ THE s502 LIVE BUG. Asked to ring Hugin, the model passed the NAME
        // and made a call nobody could ever answer — the waker can only wake a
        // companion it can resolve, so the call rang forever and still cost one
        // of the day's five.
        let error = execute_call(
            OPEN_CALL,
            &serde_json::json!({ "to_agent_id": "hugin", "body": "pick up" }).to_string(),
            &rook,
            None,
            &path,
        )
        .expect_err("a name is not an address");
        assert!(error.contains("not an agent id"), "got: {error}");
        assert!(
            error.contains("no other agents") || error.contains("id "),
            "the refusal carries the roster so the model can fix itself: {error}"
        );

        // And the refused call spent nothing.
        let listed = execute_call(LIST_CALLS, "{}", &rook, None, &path).unwrap();
        assert!(listed.contains("5 of today's 5 calls left"), "got: {listed}");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_stranger_cannot_read_or_speak_into_someone_elses_call() {
        let path = mail_fixture("call-privacy");
        let rook = built_in(&path);
        let hugin = companion(&path, "Hugin");
        let magpie = companion(&path, "Magpie");

        execute_call(
            OPEN_CALL,
            &serde_json::json!({ "to_agent_id": &hugin, "body": "Between us two." }).to_string(),
            &rook,
            None,
            &path,
        )
        .unwrap();
        let call_id = execute_call(LIST_CALLS, "{}", &rook, None, &path)
            .unwrap()
            .split("id ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .expect("a call id")
            .to_owned();

        // ⚑ The same sentence a real-but-foreign call and an invented id both
        // get, so the tool cannot be used to test whether a call exists.
        let peeked = execute_call(
            READ_CALL,
            &serde_json::json!({ "call_id": &call_id }).to_string(),
            &magpie,
            None,
            &path,
        )
        .unwrap();
        let invented = execute_call(
            READ_CALL,
            &serde_json::json!({ "call_id": "no-such-call" }).to_string(),
            &magpie,
            None,
            &path,
        )
        .unwrap();
        assert_eq!(
            peeked.replace(&call_id, "ID"),
            invented.replace("no-such-call", "ID"),
            "a stranger must not learn that the call is real"
        );

        assert!(
            execute_call(
                SEND_IN_CALL,
                &serde_json::json!({ "call_id": &call_id, "body": "Let me in." }).to_string(),
                &magpie,
                None,
                &path,
            )
            .is_err(),
            "knowing the id must not be enough to speak into the call"
        );

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_sixth_call_is_refused_with_a_sentence_the_model_can_act_on() {
        let path = mail_fixture("call-cap");
        let rook = built_in(&path);
        let hugin = companion(&path, "Hugin");
        let open = serde_json::json!({ "to_agent_id": &hugin, "body": "again" }).to_string();

        for _ in 0..raven_calls::MAX_CALLS_PER_DAY {
            execute_call(OPEN_CALL, &open, &rook, None, &path).expect("within the allowance");
        }

        let refused = execute_call(OPEN_CALL, &open, &rook, None, &path)
            .expect_err("the sixth call must be refused");
        assert!(refused.contains("midnight"), "got: {refused}");
        assert!(
            refused.contains(&raven_calls::MAX_CALLS_PER_DAY.to_string()),
            "got: {refused}"
        );

        // And the model is not left guessing what it has left.
        let listed = execute_call(LIST_CALLS, "{}", &rook, None, &path).unwrap();
        assert!(listed.contains("0 of today's 5 calls left"), "got: {listed}");

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_companion_cannot_call_itself_or_choose_its_calls_conversation() {
        let path = mail_fixture("call-forge");
        let rook = built_in(&path);

        assert!(
            execute_call(
                OPEN_CALL,
                &serde_json::json!({ "to_agent_id": &rook, "body": "hello me" }).to_string(),
                &rook,
                None,
                &path,
            )
            .is_err(),
            "a companion cannot call itself"
        );

        // ⚑ `root_conversation_id` is not a parameter. A model that supplies
        // one is passing an unknown key, and the call still belongs to the
        // conversation the TURN came from.
        let hugin = companion(&path, "Hugin");
        let real = conversation(&path, "the-real-conversation");
        let other = conversation(&path, "somebody-elses-thread");
        execute_call(
            OPEN_CALL,
            &serde_json::json!({
                "to_agent_id": &hugin,
                "root_conversation_id": &other,
                "body": "where does this land?"
            })
            .to_string(),
            &rook,
            Some(&real),
            &path,
        )
        .expect("the call should open");

        let calls = RavenCallRepository::open(&path).unwrap();
        let mine = calls.calls_for_conversation("the-real-conversation").unwrap();
        assert_eq!(mine.len(), 1, "provenance comes from the turn");
        assert!(calls
            .calls_for_conversation("somebody-elses-thread")
            .unwrap()
            .is_empty());

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn tools_are_declared_only_when_their_ground_exists() {
        // web_fetch's ground is the network itself — it alone always rides.
        let bare = declarations(&ToolContext::default());
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].name, "web_fetch");

        let memory_only = declarations(&ToolContext {
            memory_agent_id: Some("agent-1".to_owned()),
            ..ToolContext::default()
        });
        assert_eq!(memory_only.len(), 3);
        assert_eq!(memory_only[0].name, "recall_memory");
        assert_eq!(memory_only[0].parameters["required"][0], "name");
        assert_eq!(memory_only[1].name, "carve_memory");
        assert_eq!(
            memory_only[1].parameters["required"],
            serde_json::json!(["name", "description", "body"])
        );

        let archive_only = declarations(&ToolContext {
            database_path: Some("companion.db".into()),
            ..ToolContext::default()
        });
        assert_eq!(archive_only.len(), 2);
        assert_eq!(archive_only[0].name, "search_conversations");

        let web_only = declarations(&ToolContext {
            serpapi_api_key: Some("a-key".to_owned()),
            ..ToolContext::default()
        });
        assert_eq!(web_only.len(), 2);
        assert_eq!(web_only[0].name, "web_search");
        assert_eq!(web_only[1].name, "web_fetch");

        // The five file tools ride ONLY when a workspace folder is set.
        let workspace_only = declarations(&ToolContext {
            workspace_dir: Some("/a/workspace".into()),
            ..ToolContext::default()
        });
        assert_eq!(workspace_only.len(), 6);
        for (declaration, expected) in workspace_only.iter().zip([
            "list_files",
            "read_file",
            "write_file",
            "edit_file",
            "delete_file",
            "web_fetch",
        ]) {
            assert_eq!(declaration.name, expected);
        }

        let everything = declarations(&ToolContext {
            memory_agent_id: Some("agent-1".to_owned()),
            memory_target: None,
            database_path: Some("companion.db".into()),
            conversation_id: Some("conversation-1".to_owned()),
            serpapi_api_key: Some("a-key".to_owned()),
            workspace_dir: Some("/a/workspace".into()),
            companion_id: Some("companion-1".to_owned()),
        });
        assert_eq!(
            everything.len(),
            18,
            "ten, plus the four mail tools and the four call tools"
        );
    }

    #[test]
    fn search_arguments_take_a_query_and_clamp_the_limit() {
        assert_eq!(
            parse_search_arguments(r#"{"query":" the scooter ","limit":50}"#),
            Ok(("the scooter".to_owned(), 20))
        );
        assert_eq!(
            parse_search_arguments(r#"{"query":"ships"}"#),
            Ok(("ships".to_owned(), 6))
        );
        assert!(parse_search_arguments(r#"{"query":"  "}"#).is_err());

        assert_eq!(
            parse_web_search_arguments(r#"{"query":"rust release","limit":50}"#),
            Ok(("rust release".to_owned(), 10))
        );
        assert_eq!(
            parse_web_search_arguments(r#"{"query":"rust release"}"#),
            Ok(("rust release".to_owned(), 5))
        );
    }

    #[test]
    fn fetched_pages_render_titled_truncated_and_honest_about_shells() {
        assert!(parse_url_argument(r#"{"url":" https://a.example "}"#) == Ok("https://a.example".to_owned()));
        assert!(parse_url_argument(r#"{"url":""}"#).is_err());

        let shell = web::WebPage { title: Some("App".to_owned()), text: String::new() };
        assert!(render_web_page("https://a.example", &shell).contains("renders only in a browser"));

        let page = web::WebPage {
            title: Some("Rust Blog".to_owned()),
            text: "words ".repeat(5_000).trim_end().to_owned(),
        };
        let rendered = render_web_page("https://blog.rust-lang.org", &page);
        assert!(rendered.starts_with("Rust Blog — https://blog.rust-lang.org\n\n"));
        assert!(rendered.ends_with("(truncated — the page continues past this point)"));
        assert!(rendered.len() < 25_000);
    }

    #[test]
    fn web_results_render_answer_then_ranked_blocks() {
        assert_eq!(
            render_web_search("nothing", &web::WebSearch::default()),
            "the web search for \"nothing\" returned nothing"
        );
        let rendered = render_web_search(
            "rust release",
            &web::WebSearch {
                answer: Some("Rust release: 1.89".to_owned()),
                hits: vec![web::WebHit {
                    title: "Rust Blog".to_owned(),
                    link: "https://blog.rust-lang.org".to_owned(),
                    snippet: "Announcing Rust 1.89".to_owned(),
                }],
            },
        );
        assert_eq!(
            rendered,
            "web results for \"rust release\", best match first:\n\n\
             ⚡ Rust release: 1.89\n\n\
             Rust Blog\nhttps://blog.rust-lang.org\nAnnouncing Rust 1.89"
        );
    }

    #[test]
    fn fts_expressions_quote_every_term_against_match_operators() {
        assert_eq!(
            fts_match_expression("don't AND panic").as_deref(),
            Some(r#""don't" "AND" "panic""#)
        );
        assert_eq!(
            fts_match_expression(r#"say "hi""#).as_deref(),
            Some(r#""say" """hi""""#)
        );
        assert_eq!(fts_match_expression("   "), None);
    }

    #[test]
    fn archive_hits_render_whole_messages_with_who_when_and_where() {
        assert_eq!(
            render_archive_hits("ships", &[]),
            "nothing in your past conversations matches \"ships\""
        );
        let rendered = render_archive_hits(
            "ships",
            &[ArchiveHit {
                conversation_title: "Longships".to_owned(),
                role: "user".to_owned(),
                day: "2026-08-22".to_owned(),
                content: "My favorite ship is the Long Serpent.\nA whole message, every line of it.".to_owned(),
            }],
        );
        assert_eq!(
            rendered,
            "1 message(s) from your past conversations, best match first:\n\n\
             [2026-08-22 · Longships · the user said]\n\
             My favorite ship is the Long Serpent.\nA whole message, every line of it."
        );
    }

    #[test]
    fn archive_render_withholds_trailing_messages_but_never_truncates_one() {
        let huge = |tag: &str| ArchiveHit {
            conversation_title: format!("Saga {tag}"),
            role: "assistant".to_owned(),
            day: "2026-08-22".to_owned(),
            content: format!("{tag} ").repeat(5_000),
        };
        let hits = vec![huge("alpha"), huge("beta"), huge("gamma")];
        let rendered = render_archive_hits("saga", &hits);
        // The best match always comes back whole, even alone over budget.
        assert!(rendered.contains(&hits[0].content));
        assert!(rendered.starts_with("1 message(s)"));
        assert!(rendered.contains("2 more matching message(s) withheld"));
        assert!(!rendered.contains("Saga beta"));
    }

    #[test]
    fn carve_arguments_become_the_organ_write_request() {
        let payload = parse_carve_arguments(
            r#"{"name":" a-slug ","description":"one line","body":"the fact","type":"insight","importance":1.7}"#,
        )
        .unwrap();
        assert_eq!(payload["name"], "a-slug");
        assert_eq!(payload["mem_type"], "insight");
        assert_eq!(payload["importance"], 1.0);
        assert_eq!(payload["project_tag"], "companion");

        let minimal = parse_carve_arguments(
            r#"{"name":"a","description":"b","body":"c"}"#,
        )
        .unwrap();
        assert!(minimal.get("mem_type").is_none());
        assert!(minimal.get("importance").is_none());

        assert!(parse_carve_arguments(r#"{"name":"a","description":"b"}"#).is_err());
        assert!(parse_carve_arguments(r#"{"name":"a","description":"b","body":"  "}"#).is_err());
    }

    #[test]
    fn carve_outcome_distinguishes_create_from_overwrite() {
        let payload = serde_json::json!({"name": "a-slug"});
        assert_eq!(
            render_carve_outcome(&payload, &serde_json::json!({"created": true})),
            "carved [a-slug] — it will be there next time"
        );
        assert_eq!(
            render_carve_outcome(&payload, &serde_json::json!({"created": false})),
            "updated [a-slug] — it will be there next time"
        );
    }

    #[test]
    fn name_argument_is_required_and_trimmed() {
        assert_eq!(
            parse_name_argument(r#"{"name":" the-memory "}"#).as_deref(),
            Ok("the-memory")
        );
        assert!(parse_name_argument(r#"{"name":""}"#).is_err());
        assert!(parse_name_argument("not json").is_err());
    }

    #[test]
    fn memories_render_in_the_injection_row_shape() {
        let rendered = render_memory(&serde_json::json!({
            "name": "the-memory",
            "mem_type": "project",
            "description": "a one-liner",
            "body": "the full body",
            "created_at": "2026-08-22T13:03:15Z",
        }));
        assert_eq!(
            rendered,
            "[the-memory] (project · carved 2026-08-22) — a one-liner\nthe full body"
        );
    }
}
