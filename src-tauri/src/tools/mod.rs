// Companion's tool catalog — the tools the model may call natively during a
// chat turn. Declarations ride the inference request (provider tool_use, never
// XML — the s388 decision); execution happens HERE, backend-side, against the
// resources this machine holds. Tool #1 is `recall_memory`: the drill-down
// behind the ambient memory reflexes — injected bodies cap at 700 chars, this
// fetches the uncut memory from the organ on :8002. Tool #2 is `carve_memory`:
// the write half of the loop — the model commits a memory in its own words,
// freely, whenever something durable happens.

mod files;

use std::path::PathBuf;

use crate::chat::repository::{ArchiveHit, ChatRepository};
use crate::inference::{ToolCall, ToolDeclaration};
use crate::memory;
use crate::web;

pub(crate) const RECALL_MEMORY: &str = "recall_memory";
pub(crate) const CARVE_MEMORY: &str = "carve_memory";
pub(crate) const SEARCH_CONVERSATIONS: &str = "search_conversations";
pub(crate) const WEB_SEARCH: &str = "web_search";
pub(crate) const WEB_FETCH: &str = "web_fetch";

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
    /// The local chat database — ground of the raw-memory drill. The tool
    /// opens its own read connection so it never contends with the stream.
    pub(crate) archive_database_path: Option<PathBuf>,
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
    if context.archive_database_path.is_some() {
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
pub(crate) async fn execute(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    match call.name.as_str() {
        RECALL_MEMORY => {
            let agent_id = context
                .memory_agent_id
                .as_deref()
                .ok_or_else(|| "memory is not connected for this conversation".to_owned())?;
            let name = parse_name_argument(&call.arguments)?;
            let memory = memory::fetch_memory(agent_id, &name).await?;
            Ok(render_memory(&memory))
        }
        CARVE_MEMORY => {
            let agent_id = context
                .memory_agent_id
                .as_deref()
                .ok_or_else(|| "memory is not connected for this conversation".to_owned())?;
            let payload = parse_carve_arguments(&call.arguments)?;
            let result = memory::write_memory(agent_id, &payload).await?;
            Ok(render_carve_outcome(&payload, &result))
        }
        SEARCH_CONVERSATIONS => {
            let path = context
                .archive_database_path
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
    use crate::chat::repository::ArchiveHit;
    use crate::web;

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
            archive_database_path: Some("companion.db".into()),
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
            archive_database_path: Some("companion.db".into()),
            conversation_id: Some("conversation-1".to_owned()),
            serpapi_api_key: Some("a-key".to_owned()),
            workspace_dir: Some("/a/workspace".into()),
        });
        assert_eq!(everything.len(), 10);
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
