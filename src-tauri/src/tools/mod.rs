// Companion's tool catalog — the tools the model may call natively during a
// chat turn. Declarations ride the inference request (provider tool_use, never
// XML — the s388 decision); execution happens HERE, backend-side, against the
// resources this machine holds. Tool #1 is `recall_memory`: the drill-down
// behind the ambient memory reflexes — injected bodies cap at 700 chars, this
// fetches the uncut memory from the organ on :8002.

use crate::inference::{ToolCall, ToolDeclaration};
use crate::memory;

pub(crate) const RECALL_MEMORY: &str = "recall_memory";

/// Everything tool execution needs for one submission. Tools whose ground is
/// absent (no memory agent) are simply not declared, so the model never sees
/// a tool it cannot use.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolContext {
    pub(crate) memory_agent_id: Option<String>,
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
        other => Err(format!("unknown tool \"{other}\"")),
    }
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
    use super::{declarations, parse_name_argument, render_memory, ToolContext};

    #[test]
    fn tools_are_declared_only_when_their_ground_exists() {
        assert!(declarations(&ToolContext::default()).is_empty());
        let tools = declarations(&ToolContext {
            memory_agent_id: Some("agent-1".to_owned()),
        });
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "recall_memory");
        assert_eq!(tools[0].parameters["required"][0], "name");
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
