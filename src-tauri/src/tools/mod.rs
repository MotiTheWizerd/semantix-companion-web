// Companion's tool catalog — the tools the model may call natively during a
// chat turn. Declarations ride the inference request (provider tool_use, never
// XML — the s388 decision); execution happens HERE, backend-side, against the
// resources this machine holds. Tool #1 is `recall_memory`: the drill-down
// behind the ambient memory reflexes — injected bodies cap at 700 chars, this
// fetches the uncut memory from the organ on :8002. Tool #2 is `carve_memory`:
// the write half of the loop — the model commits a memory in its own words,
// freely, whenever something durable happens.

use std::path::PathBuf;

use crate::chat::repository::{ArchiveHit, ChatRepository};
use crate::inference::{ToolCall, ToolDeclaration};
use crate::memory;

pub(crate) const RECALL_MEMORY: &str = "recall_memory";
pub(crate) const CARVE_MEMORY: &str = "carve_memory";
pub(crate) const SEARCH_CONVERSATIONS: &str = "search_conversations";

const SEARCH_DEFAULT_LIMIT: u32 = 6;
const SEARCH_MAX_LIMIT: u32 = 20;

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
                "detail. Results are snippets marked »like this«, best ",
                "matches first. This is your own memory of your shared ",
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
                        "description": "Max snippets to return (default 6, max 20)."
                    }
                },
                "required": ["query"]
            }),
        });
    }
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
        other => Err(format!("unknown tool \"{other}\"")),
    }
}

fn parse_search_arguments(arguments: &str) -> Result<(String, u32), String> {
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
        .map(|limit| (limit as u32).clamp(1, SEARCH_MAX_LIMIT))
        .unwrap_or(SEARCH_DEFAULT_LIMIT);
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

/// Hits rendered for the model: one line per snippet, who said it and when.
fn render_archive_hits(query: &str, hits: &[ArchiveHit]) -> String {
    if hits.is_empty() {
        return format!("nothing in your past conversations matches \"{query}\"");
    }
    let lines: Vec<String> = hits
        .iter()
        .map(|hit| {
            let who = match hit.role.as_str() {
                "user" => "the user said",
                _ => "you said",
            };
            format!(
                "[{} · {} · {}] {}",
                hit.day, hit.conversation_title, who, hit.snippet
            )
        })
        .collect();
    format!(
        "{} moment(s) from your past conversations, best match first:\n{}",
        hits.len(),
        lines.join("\n")
    )
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
        parse_search_arguments, render_archive_hits, render_carve_outcome, render_memory,
        ToolContext,
    };
    use crate::chat::repository::ArchiveHit;

    #[test]
    fn tools_are_declared_only_when_their_ground_exists() {
        assert!(declarations(&ToolContext::default()).is_empty());

        let memory_only = declarations(&ToolContext {
            memory_agent_id: Some("agent-1".to_owned()),
            ..ToolContext::default()
        });
        assert_eq!(memory_only.len(), 2);
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
        assert_eq!(archive_only.len(), 1);
        assert_eq!(archive_only[0].name, "search_conversations");

        let everything = declarations(&ToolContext {
            memory_agent_id: Some("agent-1".to_owned()),
            archive_database_path: Some("companion.db".into()),
            conversation_id: Some("conversation-1".to_owned()),
        });
        assert_eq!(everything.len(), 3);
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
    fn archive_hits_render_who_when_and_where() {
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
                snippet: "my favorite »ship« is the Long Serpent".to_owned(),
            }],
        );
        assert_eq!(
            rendered,
            "1 moment(s) from your past conversations, best match first:\n\
             [2026-08-22 · Longships · the user said] my favorite »ship« is the Long Serpent"
        );
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
