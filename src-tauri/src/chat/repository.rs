use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{AcceptedMessage, Conversation, ConversationThread, Message, MessageAttachment};
use crate::{app_error::AppError, database};

const CONVERSATION_COLUMNS: &str =
    "id, title, companion_id, created_at, updated_at, archived_at";

pub(crate) struct ChatRepository {
    connection: Mutex<Connection>,
}

pub(crate) struct CommitUserMessage<'a> {
    pub(crate) conversation_id: Option<&'a str>,
    /// "user" for a person's message, "system" for a notice a woken turn is
    /// answering. Parameterised rather than hardcoded so a turn nobody asked
    /// for does not have to put words in the user's mouth to exist.
    pub(crate) role: &'a str,
    /// Who is answering. Stored on the conversation, so reopening a thread
    /// brings back its companion — and with it the voice and the memory.
    pub(crate) companion_id: &'a str,
    pub(crate) content: &'a str,
    pub(crate) title: &'a str,
    pub(crate) timestamp: i64,
    pub(crate) new_conversation_id: &'a str,
    pub(crate) message_id: &'a str,
    /// Images riding with the message — validated and identity-minted by the
    /// service; the repository just makes them durable.
    pub(crate) attachments: &'a [MessageAttachment],
}

/// One hit from the raw-memory drill (search_conversations tool): where the
/// match sits and the WHOLE message it sits in — a snippet proved too thin to
/// answer from, so the full text rides along and size is governed by message
/// count at render time, never by truncation.
pub(crate) struct ArchiveHit {
    pub(crate) conversation_title: String,
    pub(crate) role: String,
    pub(crate) day: String,
    pub(crate) content: String,
    /// Where the conversation came from: None = born in this app,
    /// Some("claude"/"chatgpt") = imported history.
    pub(crate) source: Option<String>,
}

/// One turn of an imported conversation, borrowed from the parsed export —
/// an import is ~100MB of text, so nothing here is cloned to be stored.
pub(crate) struct ArchivedImportTurn<'a> {
    pub(crate) role: &'a str,
    pub(crate) text: &'a str,
}

/// An imported conversation on its way into the archive as a REAL
/// conversation row: hidden from every list (`archived_at` set), fully
/// drillable by search, stamped with its ORIGINAL times so hits date true.
pub(crate) struct ArchivedImportConversation<'a> {
    /// Deterministic — "import:<source>:<source_id>" — so re-drops upsert
    /// instead of duplicating.
    pub(crate) id: String,
    pub(crate) title: &'a str,
    pub(crate) companion_id: &'a str,
    pub(crate) source: &'a str,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) turns: Vec<ArchivedImportTurn<'a>>,
}

impl ChatRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    /// Full-text search over every completed user/assistant message, best
    /// (bm25) matches first. The current conversation is excluded — the model
    /// already holds it in context; this drill is for the OTHER conversations.
    pub(crate) fn search_messages(
        &self,
        query: &str,
        exclude_conversation_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ArchiveHit>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT c.title, m.role,
                        date(m.created_at / 1000, 'unixepoch') AS day,
                        m.content, c.source
                 FROM messages_fts
                 JOIN messages m ON m.rowid = messages_fts.rowid
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE messages_fts MATCH ?1
                   AND m.status = 'completed'
                   AND m.role IN ('user', 'assistant')
                   AND (?2 IS NULL OR m.conversation_id <> ?2)
                 ORDER BY bm25(messages_fts)
                 LIMIT ?3",
            )
            .map_err(AppError::database)?;

        let hits = statement
            .query_map(params![query, exclude_conversation_id, limit], |row| {
                Ok(ArchiveHit {
                    conversation_title: row.get(0)?,
                    role: row.get(1)?,
                    day: row.get(2)?,
                    content: row.get(3)?,
                    source: row.get(4)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(hits)
    }

    /// File imported conversations into the archive as real rows — hidden
    /// from every list (`archived_at` = the import moment), indexed by the
    /// FTS triggers on insert, dated by their ORIGINAL stamps.
    ///
    /// Idempotent by deterministic id: an already-archived conversation is
    /// skipped unless the incoming `updated_at` is newer, in which case it is
    /// replaced whole (a newer export can carry more turns). Every message is
    /// born `slept_at`-stamped — the distiller meets these conversations
    /// through the import queue, never through the live sleep rail.
    ///
    /// Returns (added, refreshed).
    pub(crate) fn archive_imported_conversations(
        &self,
        records: &[ArchivedImportConversation<'_>],
        archived_at: i64,
    ) -> Result<(usize, usize), AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        let mut added = 0usize;
        let mut refreshed = 0usize;
        for record in records {
            let stored: Option<i64> = transaction
                .query_row(
                    "SELECT updated_at FROM conversations WHERE id = ?1",
                    [&record.id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(AppError::database)?;
            match stored {
                Some(stored) if stored >= record.updated_at => continue,
                Some(_) => {
                    // Explicit even though the FK cascades: the FTS delete
                    // trigger must see every message row go.
                    transaction
                        .execute(
                            "DELETE FROM messages WHERE conversation_id = ?1",
                            [&record.id],
                        )
                        .map_err(AppError::database)?;
                    transaction
                        .execute("DELETE FROM conversations WHERE id = ?1", [&record.id])
                        .map_err(AppError::database)?;
                    refreshed += 1;
                }
                None => added += 1,
            }
            transaction
                .execute(
                    "INSERT INTO conversations (
                        id, title, companion_id, created_at, updated_at, archived_at, source
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        record.id,
                        record.title,
                        record.companion_id,
                        record.created_at,
                        record.updated_at,
                        archived_at,
                        record.source,
                    ],
                )
                .map_err(AppError::database)?;
            for (sequence, turn) in record.turns.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO messages (
                            id, conversation_id, sequence, role, status, content,
                            provider_id, model_id, error_message,
                            created_at, updated_at, completed_at, slept_at
                         ) VALUES (?1, ?2, ?3, ?4, 'completed', ?5,
                                   NULL, NULL, NULL, ?6, ?6, ?6, ?6)",
                        params![
                            format!("{}#{sequence}", record.id),
                            record.id,
                            sequence as i64,
                            turn.role,
                            turn.text,
                            record.created_at,
                        ],
                    )
                    .map_err(AppError::database)?;
            }
        }
        transaction.commit().map_err(AppError::database)?;
        Ok((added, refreshed))
    }

    /// Where a woken companion should speak: the live thread it was last used
    /// in. `None` means it has none, and the caller opens a fresh one.
    pub(crate) fn latest_conversation_for_companion(
        &self,
        companion_id: &str,
    ) -> Result<Option<String>, AppError> {
        self.connection()?
            .query_row(
                "SELECT id FROM conversations
                 WHERE companion_id = ?1 AND archived_at IS NULL
                 ORDER BY updated_at DESC, id ASC
                 LIMIT 1",
                [companion_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(AppError::database)
    }

    /// Whether a conversation exists and is not archived — the same predicate
    /// `commit_user_message` enforces, checkable BEFORE preparing a whole
    /// turn, so a caller with a preferred thread can fall back instead of
    /// failing mid-wake.
    pub(crate) fn conversation_is_live(&self, conversation_id: &str) -> Result<bool, AppError> {
        self.connection()?
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM conversations
                     WHERE id = ?1 AND archived_at IS NULL
                 )",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(AppError::database)
    }

    pub(crate) fn list_conversations(&self) -> Result<Vec<Conversation>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS}
                 FROM conversations
                 WHERE archived_at IS NULL
                 ORDER BY updated_at DESC, created_at DESC"
            ))
            .map_err(AppError::database)?;

        let conversations = statement
            .query_map([], conversation_from_row)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(conversations)
    }

    pub(crate) fn fail_interrupted_streams(&self, timestamp: i64) -> Result<usize, AppError> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE messages
                 SET status = 'failed',
                     error_message = 'Response interrupted before completion.',
                     updated_at = ?1,
                     completed_at = ?1
                 WHERE role = 'assistant' AND status = 'streaming'",
                [timestamp],
            )
            .map_err(AppError::database)
    }

    pub(crate) fn get_thread(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationThread>, AppError> {
        let connection = self.connection()?;
        let conversation = connection
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                [conversation_id],
                conversation_from_row,
            )
            .optional()
            .map_err(AppError::database)?;

        let Some(conversation) = conversation else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT id, conversation_id, sequence, role, status, content,
                        provider_id, model_id, error_message, created_at, updated_at, completed_at,
                        slept_at
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY sequence ASC",
            )
            .map_err(AppError::database)?;
        let mut messages = statement
            .query_map([conversation_id], message_from_row)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        let mut attachments = attachments_for_conversation(&connection, conversation_id)?;
        for message in &mut messages {
            if let Some(list) = attachments.remove(&message.id) {
                message.attachments = list;
            }
        }

        Ok(Some(ConversationThread {
            conversation,
            messages,
        }))
    }

    pub(crate) fn commit_user_message(
        &self,
        input: CommitUserMessage<'_>,
    ) -> Result<AcceptedMessage, AppError> {
        let CommitUserMessage {
            conversation_id,
            role,
            companion_id,
            content,
            title,
            timestamp,
            new_conversation_id,
            message_id,
            attachments,
        } = input;
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;

        let conversation = if let Some(conversation_id) = conversation_id {
            let mut conversation = transaction
                .query_row(
                    &format!(
                        "SELECT {CONVERSATION_COLUMNS} FROM conversations
                         WHERE id = ?1 AND archived_at IS NULL"
                    ),
                    [conversation_id],
                    conversation_from_row,
                )
                .optional()
                .map_err(AppError::database)?
                .ok_or_else(|| AppError::validation("That conversation no longer exists."))?;
            transaction
                .execute(
                    "UPDATE conversations SET companion_id = ?2 WHERE id = ?1",
                    params![conversation_id, companion_id],
                )
                .map_err(AppError::database)?;
            conversation.companion_id = Some(companion_id.to_owned());
            conversation
        } else {
            transaction
                .execute(
                    "INSERT INTO conversations (
                        id, title, companion_id, created_at, updated_at, archived_at
                     ) VALUES (?1, ?2, ?3, ?4, ?4, NULL)",
                    params![new_conversation_id, title, companion_id, timestamp],
                )
                .map_err(AppError::database)?;

            Conversation {
                id: new_conversation_id.to_owned(),
                title: title.to_owned(),
                companion_id: Some(companion_id.to_owned()),
                created_at: timestamp,
                updated_at: timestamp,
                archived_at: None,
            }
        };

        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence) + 1, 0)
                 FROM messages
                 WHERE conversation_id = ?1",
                [&conversation.id],
                |row| row.get(0),
            )
            .map_err(AppError::database)?;

        transaction
            .execute(
                "INSERT INTO messages (
                    id, conversation_id, sequence, role, status, content,
                    provider_id, model_id, error_message, created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?6, 'completed', ?4, NULL, NULL, NULL, ?5, ?5, ?5)",
                params![message_id, conversation.id, sequence, content, timestamp, role],
            )
            .map_err(AppError::database)?;
        for attachment in attachments {
            transaction
                .execute(
                    "INSERT INTO message_attachments (
                        id, message_id, media_type, data, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        attachment.id,
                        message_id,
                        attachment.media_type,
                        attachment.data,
                        timestamp
                    ],
                )
                .map_err(AppError::database)?;
        }
        transaction
            .execute(
                "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
                params![conversation.id, timestamp],
            )
            .map_err(AppError::database)?;

        transaction.commit().map_err(AppError::database)?;

        Ok(AcceptedMessage {
            conversation: Conversation {
                updated_at: timestamp,
                ..conversation
            },
            message: Message {
                id: message_id.to_owned(),
                conversation_id: conversation_id.unwrap_or(new_conversation_id).to_owned(),
                sequence,
                role: role.to_owned(),
                status: "completed".to_owned(),
                content: content.to_owned(),
                provider_id: None,
                model_id: None,
                error_message: None,
                created_at: timestamp,
                updated_at: timestamp,
                completed_at: Some(timestamp),
                slept_at: None,
                attachments: attachments.to_vec(),
            },
        })
    }

    pub(crate) fn update_companion(
        &self,
        conversation_id: &str,
        companion_id: &str,
    ) -> Result<Conversation, AppError> {
        let connection = self.connection()?;
        let changed = connection
            .execute(
                "UPDATE conversations
                 SET companion_id = ?2
                 WHERE id = ?1 AND archived_at IS NULL",
                params![conversation_id, companion_id],
            )
            .map_err(AppError::database)?;
        if changed == 0 {
            return Err(AppError::validation("That conversation no longer exists."));
        }
        connection
            .query_row(
                &format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"),
                [conversation_id],
                conversation_from_row,
            )
            .map_err(AppError::database)
    }

    pub(crate) fn begin_assistant_message(
        &self,
        conversation_id: &str,
        message_id: &str,
        provider_id: &str,
        model_id: &str,
        timestamp: i64,
    ) -> Result<Message, AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;

        let conversation_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM conversations WHERE id = ?1 AND archived_at IS NULL
                 )",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(AppError::database)?;
        if !conversation_exists {
            return Err(AppError::validation("That conversation no longer exists."));
        }

        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence) + 1, 0)
                 FROM messages
                 WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(AppError::database)?;

        transaction
            .execute(
                "INSERT INTO messages (
                    id, conversation_id, sequence, role, status, content,
                    provider_id, model_id, error_message, created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, 'assistant', 'streaming', '', ?4, ?5, NULL, ?6, ?6, NULL)",
                params![
                    message_id,
                    conversation_id,
                    sequence,
                    provider_id,
                    model_id,
                    timestamp
                ],
            )
            .map_err(AppError::database)?;
        transaction
            .execute(
                "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
                params![conversation_id, timestamp],
            )
            .map_err(AppError::database)?;
        transaction.commit().map_err(AppError::database)?;

        Ok(Message {
            id: message_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            sequence,
            role: "assistant".to_owned(),
            status: "streaming".to_owned(),
            content: String::new(),
            provider_id: Some(provider_id.to_owned()),
            model_id: Some(model_id.to_owned()),
            error_message: None,
            created_at: timestamp,
            updated_at: timestamp,
            completed_at: None,
            slept_at: None,
            attachments: Vec::new(),
        })
    }

    pub(crate) fn complete_assistant_message(
        &self,
        message_id: &str,
        content: &str,
        timestamp: i64,
    ) -> Result<Message, AppError> {
        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE messages
                 SET status = 'completed', content = ?2, error_message = NULL,
                     updated_at = ?3, completed_at = ?3
                 WHERE id = ?1 AND role = 'assistant' AND status = 'streaming'",
                params![message_id, content, timestamp],
            )
            .map_err(AppError::database)?;
        if updated != 1 {
            return Err(AppError::internal(
                "the assistant message was not in a streamable state",
            ));
        }
        message_by_id(&connection, message_id)?.ok_or_else(|| {
            AppError::internal("the completed assistant message could not be reloaded")
        })
    }

    pub(crate) fn fail_assistant_message(
        &self,
        message_id: &str,
        error_message: &str,
        timestamp: i64,
    ) -> Result<Message, AppError> {
        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE messages
                 SET status = 'failed', error_message = ?2,
                     updated_at = ?3, completed_at = ?3
                 WHERE id = ?1 AND role = 'assistant' AND status = 'streaming'",
                params![message_id, error_message, timestamp],
            )
            .map_err(AppError::database)?;
        if updated != 1 {
            return Err(AppError::internal(
                "the assistant message was not in a streamable state",
            ));
        }
        message_by_id(&connection, message_id)?
            .ok_or_else(|| AppError::internal("the failed assistant message could not be reloaded"))
    }

    /// Stamp the sleep ledger: these messages were distilled into memory, so
    /// the next /sleep pass skips them. Called only after the organ confirms
    /// the pass landed — a failed pass leaves the rows free for retry.
    /// How many conversational turns the sleep ledger has not claimed yet —
    /// the sleeper's ripeness read, same filter `prepare_sleep` distils by,
    /// without loading the thread.
    pub(crate) fn count_unslept(&self, conversation_id: &str) -> Result<usize, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE conversation_id = ?1
                   AND slept_at IS NULL
                   AND status = 'completed'
                   AND role IN ('user', 'assistant')
                   AND length(trim(content)) > 0",
                [conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(AppError::database)
    }

    pub(crate) fn mark_messages_slept(
        &self,
        message_ids: &[String],
        timestamp: i64,
    ) -> Result<(), AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        {
            let mut statement = transaction
                .prepare("UPDATE messages SET slept_at = ?2 WHERE id = ?1")
                .map_err(AppError::database)?;
            for id in message_ids {
                statement
                    .execute(params![id, timestamp])
                    .map_err(AppError::database)?;
            }
        }
        transaction.commit().map_err(AppError::database)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}

fn conversation_from_row(row: &Row<'_>) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: row.get(0)?,
        title: row.get(1)?,
        companion_id: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        archived_at: row.get(5)?,
    })
}

fn message_from_row(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        sequence: row.get(2)?,
        role: row.get(3)?,
        status: row.get(4)?,
        content: row.get(5)?,
        provider_id: row.get(6)?,
        model_id: row.get(7)?,
        error_message: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
        slept_at: row.get(12)?,
        attachments: Vec::new(),
    })
}

/// Every attachment in the thread, grouped by message — one query for the
/// whole stitch instead of one per message.
fn attachments_for_conversation(
    connection: &Connection,
    conversation_id: &str,
) -> Result<HashMap<String, Vec<MessageAttachment>>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT a.message_id, a.id, a.media_type, a.data
             FROM message_attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.conversation_id = ?1
             ORDER BY a.created_at ASC, a.id ASC",
        )
        .map_err(AppError::database)?;
    let rows = statement
        .query_map([conversation_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                MessageAttachment {
                    id: row.get(1)?,
                    media_type: row.get(2)?,
                    data: row.get(3)?,
                },
            ))
        })
        .map_err(AppError::database)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::database)?;

    let mut grouped: HashMap<String, Vec<MessageAttachment>> = HashMap::new();
    for (message_id, attachment) in rows {
        grouped.entry(message_id).or_default().push(attachment);
    }
    Ok(grouped)
}

fn message_by_id(connection: &Connection, message_id: &str) -> Result<Option<Message>, AppError> {
    connection
        .query_row(
            "SELECT id, conversation_id, sequence, role, status, content,
                    provider_id, model_id, error_message, created_at, updated_at, completed_at,
                    slept_at
             FROM messages
             WHERE id = ?1",
            [message_id],
            message_from_row,
        )
        .optional()
        .map_err(AppError::database)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ChatRepository, CommitUserMessage};
    use crate::database;

    fn open_repository(tag: &str) -> (ChatRepository, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "companion-archive-test-{tag}-{}.db",
            uuid::Uuid::new_v4()
        ));
        database::initialise(&path).expect("test database should initialise");
        (ChatRepository::open(&path).expect("repository should open"), path)
    }

    /// The built-in companion's id is a uuid minted at migration time, so a
    /// fixture has to look it up rather than name it. Nothing may hardcode it —
    /// that is the whole point of schema 12.
    fn built_in_id(path: &std::path::Path) -> String {
        rusqlite::Connection::open(path)
            .expect("test database should open")
            .query_row("SELECT id FROM companions WHERE is_built_in = 1", [], |row| {
                row.get(0)
            })
            .expect("the built-in companion should exist")
    }

    fn seed_conversation(
        repository: &ChatRepository,
        companion_id: &str,
        tag: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> String {
        let conversation_id = format!("conversation-{tag}");
        repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: None,
                companion_id,
                content: user_text,
                title: format!("Talk about {tag}").as_str(),
                timestamp: 1_755_800_000_000,
                new_conversation_id: &conversation_id,
                message_id: &format!("message-{tag}-user"),
                attachments: &[],
            })
            .expect("user message should commit");
        repository
            .begin_assistant_message(
                &conversation_id,
                &format!("message-{tag}-assistant"),
                "test",
                "test-model",
                1_755_800_001_000,
            )
            .expect("assistant message should begin");
        repository
            .complete_assistant_message(
                &format!("message-{tag}-assistant"),
                assistant_text,
                1_755_800_002_000,
            )
            .expect("assistant message should complete");
        conversation_id
    }

    #[test]
    fn the_raw_memory_drill_finds_ranks_and_excludes() {
        let (repository, path) = open_repository("drill");
        let companion_id = built_in_id(&path);
        let ships = seed_conversation(
            &repository,
            &companion_id,
            "ships",
            "My favorite ship is the Long Serpent.",
            "The Long Serpent was Olaf Tryggvason's flagship.",
        );
        let current = seed_conversation(
            &repository,
            &companion_id,
            "current",
            "The Long Serpent again, but from the conversation being excluded.",
            "Understood.",
        );

        let hits = repository
            .search_messages("\"serpent\"", Some(&current), 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 2, "only the OTHER conversation's messages match");
        assert!(hits.iter().all(|hit| hit.conversation_title == "Talk about ships"));
        let contents: Vec<&str> = hits.iter().map(|hit| hit.content.as_str()).collect();
        assert!(
            contents.contains(&"My favorite ship is the Long Serpent."),
            "the WHOLE message comes back, not a snippet: {contents:?}"
        );
        assert_eq!(hits[0].day, "2025-08-21");
        let roles: Vec<&str> = hits.iter().map(|hit| hit.role.as_str()).collect();
        assert!(roles.contains(&"user") && roles.contains(&"assistant"));

        // A message still streaming (never completed) must be invisible.
        repository
            .begin_assistant_message(&ships, "message-ships-streaming", "test", "test-model", 1_755_800_003_000)
            .expect("streaming message should begin");
        let hits = repository
            .search_messages("\"serpent\"", None, 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 3, "both conversations, completed messages only");

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// The sleeper's ripeness read counts exactly what a pass would distil:
    /// completed, non-empty user/assistant turns the ledger has not claimed.
    /// A streaming reply is not a turn yet; a stamped one is not fresh.
    #[test]
    fn the_sleeper_counts_only_fresh_finished_turns() {
        let (repository, path) = open_repository("ripeness");
        let companion_id = built_in_id(&path);
        let conversation = seed_conversation(
            &repository,
            &companion_id,
            "ripe",
            "Remember that the harbour freezes in January.",
            "Noted — January, the harbour.",
        );
        assert_eq!(repository.count_unslept(&conversation).unwrap(), 2);

        // A reply still streaming does not count until it lands.
        repository
            .begin_assistant_message(&conversation, "message-ripe-open", "test", "test-model", 1_755_800_003_000)
            .expect("streaming message should begin");
        assert_eq!(repository.count_unslept(&conversation).unwrap(), 2);
        repository
            .complete_assistant_message("message-ripe-open", "And the fjord in February.", 1_755_800_004_000)
            .expect("assistant message should complete");
        assert_eq!(repository.count_unslept(&conversation).unwrap(), 3);

        // Stamped turns leave the count; the ledger is the truth.
        repository
            .mark_messages_slept(
                &["message-ripe-user".to_owned(), "message-ripe-assistant".to_owned()],
                1_755_800_005_000,
            )
            .expect("ledger should stamp");
        assert_eq!(repository.count_unslept(&conversation).unwrap(), 1);
        assert_eq!(repository.count_unslept("conversation-nowhere").unwrap(), 0);

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// The import archive pass: filed conversations are hidden from every
    /// list, fully drillable with their origin and ORIGINAL dates, and a
    /// re-drop upserts — same export skips, a newer one replaces whole.
    #[test]
    fn imported_history_is_hidden_searchable_and_upserts_by_id() {
        let (repository, path) = open_repository("import-archive");
        let companion_id = built_in_id(&path);
        let created = 1_730_764_800_000; // 2024-11-05
        let record = |updated: i64, turns: &'static [(&'static str, &'static str)]| {
            super::ArchivedImportConversation {
                id: "import:claude:conv-1".to_owned(),
                title: "Discussing DMT Responsibly",
                companion_id: &companion_id,
                source: "claude",
                created_at: created,
                updated_at: updated,
                turns: turns
                    .iter()
                    .map(|(role, text)| super::ArchivedImportTurn { role, text })
                    .collect(),
            }
        };
        const ORIGINAL: [(&str, &str); 2] = [
            ("user", "Is DMT a shift of frequency to another layer?"),
            ("assistant", "Casually put: DMT research says the mind does it."),
        ];

        let filed = repository
            .archive_imported_conversations(&[record(created, &ORIGINAL)], 1_756_500_000_000)
            .expect("the archive pass should succeed");
        assert_eq!(filed, (1, 0));
        assert!(
            repository
                .list_conversations()
                .expect("listing should succeed")
                .iter()
                .all(|conversation| conversation.id != "import:claude:conv-1"),
            "an imported conversation never appears in the sidebar"
        );

        let hits = repository
            .search_messages("\"dmt\"", None, 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 2, "both imported turns are drillable");
        assert!(hits.iter().all(|hit| hit.source.as_deref() == Some("claude")));
        assert_eq!(hits[0].day, "2024-11-05", "hits date by the ORIGINAL stamp");

        // The same export again: nothing added, nothing duplicated.
        let filed = repository
            .archive_imported_conversations(&[record(created, &ORIGINAL)], 1_756_500_001_000)
            .expect("the re-drop should succeed");
        assert_eq!(filed, (0, 0));
        let hits = repository
            .search_messages("\"dmt\"", None, 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 2, "a re-drop never duplicates messages");

        // A NEWER export of the same conversation replaces it whole.
        const GROWN: [(&str, &str); 3] = [
            ("user", "Is DMT a shift of frequency to another layer?"),
            ("assistant", "Casually put: DMT research says the mind does it."),
            ("user", "One more DMT question then."),
        ];
        let filed = repository
            .archive_imported_conversations(
                &[record(created + 60_000, &GROWN)],
                1_756_500_002_000,
            )
            .expect("the newer drop should succeed");
        assert_eq!(filed, (0, 1));
        let hits = repository
            .search_messages("\"dmt\"", None, 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 3, "the refreshed conversation carries its new turn");

        drop(repository);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn attachments_survive_the_round_trip_and_ride_only_their_own_message() {
        let (repository, path) = open_repository("attachments");
        let companion_id = built_in_id(&path);
        let companion_id = companion_id.as_str();
        let attachments = vec![crate::chat::MessageAttachment {
            id: "attachment-1".to_owned(),
            media_type: "image/png".to_owned(),
            data: "aGk=".to_owned(),
        }];
        let accepted = repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: None,
                companion_id,
                content: "Look at this.",
                title: "Look at this.",
                timestamp: 1_755_800_000_000,
                new_conversation_id: "conversation-images",
                message_id: "message-with-image",
                attachments: &attachments,
            })
            .expect("user message should commit");
        assert_eq!(accepted.message.attachments.len(), 1, "the echo carries the image");

        // A second, imageless message in the same thread.
        repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: Some("conversation-images"),
                companion_id,
                content: "And a plain one.",
                title: "Look at this.",
                timestamp: 1_755_800_001_000,
                new_conversation_id: "unused",
                message_id: "message-plain",
                attachments: &[],
            })
            .expect("plain message should commit");

        let thread = repository
            .get_thread("conversation-images")
            .expect("thread should load")
            .expect("thread should exist");
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].attachments.len(), 1);
        assert_eq!(thread.messages[0].attachments[0].media_type, "image/png");
        assert_eq!(thread.messages[0].attachments[0].data, "aGk=");
        assert!(thread.messages[1].attachments.is_empty());

        drop(repository);
        let _ = fs::remove_file(path);
    }
}
