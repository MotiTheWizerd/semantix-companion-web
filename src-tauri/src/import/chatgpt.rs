// The ChatGPT export — `conversations.json`, or `conversations-NNN.json`
// shards in newer exports. A conversation is not a list but a TREE: every
// edit or regeneration forked a branch, and `mapping` keeps them all. The
// transcript the user actually saw is the chain from `current_node` back to
// the root — so the parser walks up the parent links and reverses, and the
// abandoned branches die unread. Segmenting any other way would import
// wordings the user rejected.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::{ImportTurn, ImportedConversation, TurnRole};

#[derive(Deserialize)]
struct Conversation {
    #[serde(default)]
    id: String,
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    create_time: Option<f64>,
    #[serde(default)]
    update_time: Option<f64>,
    #[serde(default)]
    current_node: Option<String>,
    #[serde(default)]
    mapping: HashMap<String, Node>,
}

#[derive(Deserialize)]
struct Node {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    author: Author,
    #[serde(default)]
    content: Option<Content>,
    #[serde(default)]
    create_time: Option<f64>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct Author {
    #[serde(default)]
    role: String,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    parts: Vec<serde_json::Value>,
}

/// Parse one ChatGPT conversations file (whole export or one shard).
pub(super) fn parse_conversations(
    bytes: &[u8],
) -> Result<(Vec<ImportedConversation>, usize), String> {
    let conversations: Vec<Conversation> = serde_json::from_slice(bytes)
        .map_err(|error| format!("The ChatGPT export could not be read: {error}"))?;

    let mut parsed = Vec::with_capacity(conversations.len());
    let mut empty_skipped = 0usize;
    for conversation in conversations {
        let turns = transcript_of(&conversation);
        if turns.is_empty() {
            empty_skipped += 1;
            continue;
        }
        let source_id = if conversation.conversation_id.is_empty() {
            conversation.id
        } else {
            conversation.conversation_id
        };
        parsed.push(ImportedConversation {
            source_id,
            title: conversation.title.unwrap_or_default(),
            created_at_ms: epoch_ms(conversation.create_time),
            updated_at_ms: epoch_ms(conversation.update_time),
            turns,
        });
    }
    Ok((parsed, empty_skipped))
}

fn epoch_ms(seconds: Option<f64>) -> i64 {
    seconds.map(|s| (s * 1000.0) as i64).unwrap_or(0)
}

fn transcript_of(conversation: &Conversation) -> Vec<ImportTurn> {
    let mapping = &conversation.mapping;
    let start = conversation
        .current_node
        .as_deref()
        .filter(|id| mapping.contains_key(*id))
        .map(str::to_owned)
        .or_else(|| latest_leaf(mapping));
    let Some(start) = start else { return Vec::new() };

    // Walk up the chosen branch; the visited set turns a corrupt cycle into a
    // short transcript instead of a hung import.
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    let mut cursor = Some(start);
    while let Some(id) = cursor {
        if !visited.insert(id.clone()) {
            break;
        }
        let Some(node) = mapping.get(&id) else { break };
        chain.push(node);
        cursor = node.parent.clone();
    }
    chain.reverse();

    chain.iter().filter_map(|node| turn_of(node)).collect()
}

/// When `current_node` is missing or dangling, the freshest leaf is the best
/// guess at the branch the user last stood on.
fn latest_leaf(mapping: &HashMap<String, Node>) -> Option<String> {
    let parents: HashSet<&str> = mapping
        .values()
        .filter_map(|node| node.parent.as_deref())
        .collect();
    mapping
        .iter()
        .filter(|(id, _)| !parents.contains(id.as_str()))
        .max_by(|(_, a), (_, b)| {
            let stamp = |n: &Node| n.message.as_ref().and_then(|m| m.create_time).unwrap_or(0.0);
            stamp(a).total_cmp(&stamp(b))
        })
        .map(|(id, _)| id.clone())
}

fn turn_of(node: &Node) -> Option<ImportTurn> {
    let message = node.message.as_ref()?;
    let role = match message.author.role.as_str() {
        "user" => TurnRole::User,
        "assistant" => TurnRole::Assistant,
        // system prompts, tool frames, browsing results — machinery, not talk.
        _ => return None,
    };
    if message
        .metadata
        .get("is_visually_hidden_from_conversation")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    let content = message.content.as_ref()?;
    if !matches!(content.content_type.as_str(), "text" | "multimodal_text") {
        return None;
    }
    // Parts are strings, or small objects for spoken/image frames — of those
    // only ones carrying a `text` (voice transcriptions) say anything.
    let text = content
        .parts
        .iter()
        .filter_map(|part| match part {
            serde_json::Value::String(text) => Some(text.as_str()),
            other => other.get("text").and_then(serde_json::Value::as_str),
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.is_empty() {
        return None;
    }
    Some(ImportTurn { role, text })
}

#[cfg(test)]
mod tests {
    use super::super::TurnRole;
    use super::parse_conversations;

    /// root → user → assistant(kept) with an abandoned sibling branch and a
    /// tool frame in the live one. Only the current branch, only real speech.
    const FIXTURE: &str = r#"[
        {
            "conversation_id": "gpt-1", "title": "Branching chat",
            "create_time": 1735757068.6, "update_time": 1735757439.2,
            "current_node": "leaf-b",
            "mapping": {
                "root":   {"id": "root", "parent": null, "children": ["u1"], "message": null},
                "u1":     {"id": "u1", "parent": "root", "children": ["a-old", "tool"], "message":
                    {"author": {"role": "user"}, "content": {"content_type": "text", "parts": ["hello there"]}}},
                "a-old":  {"id": "a-old", "parent": "u1", "children": [], "message":
                    {"author": {"role": "assistant"}, "content": {"content_type": "text", "parts": ["rejected draft"]}}},
                "tool":   {"id": "tool", "parent": "u1", "children": ["leaf-b"], "message":
                    {"author": {"role": "tool"}, "content": {"content_type": "text", "parts": ["browser noise"]}}},
                "leaf-b": {"id": "leaf-b", "parent": "tool", "children": [], "message":
                    {"author": {"role": "assistant"}, "create_time": 1735757439.0, "content": {"content_type": "multimodal_text",
                        "parts": ["kept answer", {"content_type": "audio_transcription", "text": "spoken bit"}, {"content_type": "image_asset_pointer"}]}}}
            }
        },
        {
            "conversation_id": "gpt-empty", "title": "Only machinery",
            "create_time": 1700000000.0, "update_time": 1700000000.0,
            "current_node": "s1",
            "mapping": {
                "s1": {"id": "s1", "parent": null, "children": [], "message":
                    {"author": {"role": "system"}, "content": {"content_type": "text", "parts": ["system prompt"]}}}
            }
        }
    ]"#;

    #[test]
    fn only_the_current_branch_survives() {
        let (conversations, empty_skipped) =
            parse_conversations(FIXTURE.as_bytes()).expect("parses");

        assert_eq!(conversations.len(), 1);
        assert_eq!(empty_skipped, 1, "the machinery-only chat is skipped");
        let chat = &conversations[0];
        assert_eq!(chat.source_id, "gpt-1");
        assert_eq!(chat.created_at_ms, 1735757068600);
        assert_eq!(chat.turns.len(), 2, "tool frame and rejected draft die");
        assert_eq!(chat.turns[0].role, TurnRole::User);
        assert_eq!(chat.turns[1].role, TurnRole::Assistant);
        assert_eq!(
            chat.turns[1].text, "kept answer\nspoken bit",
            "string parts and transcription text join; image pointers vanish"
        );
    }

    /// A dangling `current_node` must degrade to the freshest leaf, not to an
    /// empty import.
    #[test]
    fn a_dangling_current_node_falls_back_to_the_freshest_leaf() {
        let fixture = FIXTURE.replace("\"current_node\": \"leaf-b\"", "\"current_node\": \"gone\"");

        let (conversations, _) = parse_conversations(fixture.as_bytes()).expect("parses");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].turns.len(), 2);
        assert!(
            conversations[0].turns[1].text.starts_with("kept answer"),
            "the stamped leaf outranks the abandoned branch"
        );
    }
}
