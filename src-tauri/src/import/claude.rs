// The Claude.ai export — one `conversations.json`, a flat array of
// conversations each carrying its whole `chat_messages` list in speech order.
//
// Only `text` blocks are conversation; `thinking`, `tool_use`, `tool_result`
// and friends are the assistant's scaffolding and would teach the distiller
// nothing about the USER (measured on the real corpus: 25,010 text blocks
// beside 5,463 thinking and ~2,000 tool frames). Attachments and files are
// skipped in this round — their `extracted_content` can dwarf the chat itself.

use serde::Deserialize;

use super::{iso_to_epoch_ms, ImportTurn, ImportedConversation, TurnRole};

#[derive(Deserialize)]
struct Conversation {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    chat_messages: Vec<Message>,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    sender: String,
    // The pre-blocks era put the whole message here; modern exports keep it as
    // a joined mirror of the blocks. It is the fallback, never the primary.
    #[serde(default)]
    text: String,
    #[serde(default)]
    content: Vec<Block>,
}

#[derive(Deserialize)]
struct Block {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

/// Parse a Claude `conversations.json`. Conversations that end up with no
/// speakable turns are counted, not returned — an empty chat is a fact about
/// the export, not an error.
pub(super) fn parse_conversations(
    bytes: &[u8],
) -> Result<(Vec<ImportedConversation>, usize), String> {
    let conversations: Vec<Conversation> = serde_json::from_slice(bytes)
        .map_err(|error| format!("The Claude export could not be read: {error}"))?;

    let mut parsed = Vec::with_capacity(conversations.len());
    let mut empty_skipped = 0usize;
    for conversation in conversations {
        let turns: Vec<ImportTurn> = conversation
            .chat_messages
            .iter()
            .filter_map(turn_of)
            .collect();
        if turns.is_empty() {
            empty_skipped += 1;
            continue;
        }
        parsed.push(ImportedConversation {
            source_id: conversation.uuid,
            title: conversation.name,
            created_at_ms: iso_to_epoch_ms(&conversation.created_at).unwrap_or(0),
            updated_at_ms: iso_to_epoch_ms(&conversation.updated_at).unwrap_or(0),
            turns,
        });
    }
    Ok((parsed, empty_skipped))
}

fn turn_of(message: &Message) -> Option<ImportTurn> {
    let role = match message.sender.as_str() {
        "human" => TurnRole::User,
        "assistant" => TurnRole::Assistant,
        _ => return None,
    };
    let mut text = message
        .content
        .iter()
        .filter(|block| block.kind == "text" && !block.text.trim().is_empty())
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.trim().is_empty() {
        text = message.text.trim().to_owned();
    }
    if text.is_empty() {
        return None;
    }
    Some(ImportTurn { role, text })
}

/// `memories.json` — Claude.ai's own distilled memory of this user, the one
/// part of the export that is ALREADY memory-shaped. Shape observed on the
/// real file: a list of objects whose string values are markdown blobs
/// (`conversations_memory`, and whatever siblings later exports grow). Every
/// non-empty string value is kept; unknown keys are a harvest, not a hazard.
pub(super) fn parse_memories(bytes: &[u8]) -> Result<Vec<String>, String> {
    let items: Vec<serde_json::Map<String, serde_json::Value>> = serde_json::from_slice(bytes)
        .map_err(|error| format!("The export's memories.json could not be read: {error}"))?;
    Ok(items
        .iter()
        .flat_map(|item| item.values())
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::super::TurnRole;
    use super::{parse_conversations, parse_memories};

    const FIXTURE: &str = r#"[
        {
            "uuid": "conv-2", "name": "Later chat",
            "created_at": "2026-01-02T10:00:00.500Z",
            "updated_at": "2026-01-02T11:00:00Z",
            "chat_messages": [
                {"sender": "human", "text": "", "content": [
                    {"type": "text", "text": "shalom"}
                ]},
                {"sender": "assistant", "content": [
                    {"type": "thinking", "text": "private reasoning"},
                    {"type": "text", "text": "first part"},
                    {"type": "tool_use", "text": ""},
                    {"type": "text", "text": "second part"}
                ]}
            ]
        },
        {
            "uuid": "conv-empty", "name": "Nothing here",
            "created_at": "2024-05-05T00:00:00Z", "updated_at": "2024-05-05T00:00:00Z",
            "chat_messages": []
        },
        {
            "uuid": "conv-legacy", "name": "Pre-blocks era",
            "created_at": "2023-07-19T23:53:17.167873Z",
            "updated_at": "2023-07-20T00:00:00+00:00",
            "chat_messages": [
                {"sender": "human", "text": "old style body", "content": []}
            ]
        }
    ]"#;

    #[test]
    fn text_blocks_join_and_scaffolding_blocks_die() {
        let (conversations, _) = parse_conversations(FIXTURE.as_bytes()).expect("parses");
        let later = conversations
            .iter()
            .find(|c| c.source_id == "conv-2")
            .expect("conv-2 survives");

        assert_eq!(later.turns.len(), 2);
        assert_eq!(later.turns[0].role, TurnRole::User);
        assert_eq!(later.turns[0].text, "shalom");
        assert_eq!(
            later.turns[1].text, "first part\n\nsecond part",
            "thinking and tool blocks must not leak into the transcript"
        );
    }

    #[test]
    fn an_empty_conversation_is_counted_not_kept() {
        let (conversations, empty_skipped) =
            parse_conversations(FIXTURE.as_bytes()).expect("parses");

        assert_eq!(conversations.len(), 2);
        assert_eq!(empty_skipped, 1);
        assert!(conversations.iter().all(|c| c.source_id != "conv-empty"));
    }

    /// The 2023 half of the corpus predates content blocks; its words live in
    /// the flat `text` field and must not be lost to modern assumptions.
    #[test]
    fn a_pre_blocks_message_falls_back_to_its_text_field() {
        let (conversations, _) = parse_conversations(FIXTURE.as_bytes()).expect("parses");
        let legacy = conversations
            .iter()
            .find(|c| c.source_id == "conv-legacy")
            .expect("legacy conv survives");

        assert_eq!(legacy.turns[0].text, "old style body");
        assert!(legacy.created_at_ms > 0, "a Z-suffixed ISO stamp must parse");
    }

    #[test]
    fn memories_harvest_every_string_value() {
        let bytes = br#"[{"conversations_memory": "**Work**\n\nfacts", "projects_memory": "more", "count": 3, "empty": "  "}]"#;

        let memories = parse_memories(bytes).expect("parses");

        assert_eq!(memories, vec!["**Work**\n\nfacts".to_owned(), "more".to_owned()]);
    }
}
