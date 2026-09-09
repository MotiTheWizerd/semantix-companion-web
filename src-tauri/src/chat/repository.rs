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

/// Read in this order by `message_from_row` — keep the two in step.
const MESSAGE_COLUMNS: &str = "id, conversation_id, sequence, role, status, content,
     provider_id, model_id, error_message, created_at, updated_at, completed_at,
     slept_at, companion_id";

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

/// The two facts the companion lock turns on (s569): who a conversation
/// answers to, and whether a single message has landed in it yet. A thread
/// nobody has spoken in is still anyone's; the first message settles it.
pub(crate) struct Ownership {
    pub(crate) companion_id: Option<String>,
    pub(crate) spoken: bool,
}

/// One hit from the raw-memory drill (search_conversations tool): where the
/// match sits and the WHOLE message it sits in — a snippet proved too thin to
/// answer from, so the full text rides along and size is governed by message
/// count at render time, never by truncation.
pub(crate) struct ArchiveHit {
    /// Where the hit sits — named in the render so the model can read the
    /// turns around it with `read_conversation`. A search finds a sentence;
    /// the moment is the stretch of turns it sits in (s559).
    pub(crate) conversation_id: String,
    pub(crate) sequence: i64,
    pub(crate) conversation_title: String,
    pub(crate) role: String,
    pub(crate) day: String,
    pub(crate) content: String,
    /// Where the conversation came from: None = born in this app,
    /// Some("claude"/"chatgpt") = imported history.
    pub(crate) source: Option<String>,
    /// The hit sits earlier in the conversation the drill was run from —
    /// rendered as such, so the model knows it is reading its own thread
    /// rather than another one.
    pub(crate) in_this_conversation: bool,
}

/// A stretch of one conversation read back in order by `read_turns` — the
/// turns around a drill hit. Text only; an image is counted, not carried.
pub(crate) struct ArchiveWindow {
    pub(crate) conversation_title: String,
    pub(crate) source: Option<String>,
    /// The conversation's last sequence number, so the render can say where
    /// the window sits ("turns 4-13 of 0-52") and the model can page on.
    pub(crate) last_sequence: i64,
    pub(crate) turns: Vec<ArchiveTurn>,
}

pub(crate) struct ArchiveTurn {
    pub(crate) sequence: i64,
    pub(crate) role: String,
    pub(crate) day: String,
    pub(crate) content: String,
    pub(crate) image_count: i64,
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

    /// Full-text search over every completed user/assistant message the
    /// given companion has had, best (bm25) matches first. The drill is ONE
    /// companion's raw memory: a message belongs to the companion it was said
    /// by or to, and no companion can read another's — the same wall the
    /// UNIQUE memory_agent_name puts around their distilled memories (s491),
    /// drawn here around the raw ones (s541: Rook could drill Hugin's past).
    /// The wall is the ROW's companion, not the thread's (s564): the thread
    /// label follows the picker, so a thread re-pointed from Rook to Qwen
    /// would have handed Qwen every word Rook said in it.
    /// The conversation the drill runs from is searched too, all but its LIVE
    /// turn (the latest user message and whatever followed it — the one part
    /// the model is guaranteed to be holding). It used to be excluded whole,
    /// on the theory that the model already had it in context. s559 proved
    /// the theory wrong: Hugin, forty turns past the exchange where its avatar
    /// was born, drilled for that exchange from inside the same thread and was
    /// told nothing matched. A resumed session compacts; the archive does
    /// not. A companion's own earlier turns are the FIRST thing it should be
    /// able to reach, not the one thing it cannot.
    pub(crate) fn search_messages(
        &self,
        query: &str,
        companion_id: &str,
        current_conversation_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ArchiveHit>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT c.title, m.role,
                        date(m.created_at / 1000, 'unixepoch') AS day,
                        m.content, c.source,
                        (?3 IS NOT NULL AND m.conversation_id = ?3) AS in_this_conversation,
                        m.conversation_id, m.sequence
                 FROM messages_fts
                 JOIN messages m ON m.rowid = messages_fts.rowid
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE messages_fts MATCH ?1
                   AND COALESCE(m.companion_id, c.companion_id) = ?2
                   AND m.status = 'completed'
                   AND m.role IN ('user', 'assistant')
                   AND (?3 IS NULL
                        OR m.conversation_id <> ?3
                        OR m.sequence < (SELECT COALESCE(MAX(sequence), 0)
                                         FROM messages
                                         WHERE conversation_id = ?3 AND role = 'user'))
                 ORDER BY bm25(messages_fts)
                 LIMIT ?4",
            )
            .map_err(AppError::database)?;

        let hits = statement
            .query_map(params![query, companion_id, current_conversation_id, limit], |row| {
                Ok(ArchiveHit {
                    conversation_title: row.get(0)?,
                    role: row.get(1)?,
                    day: row.get(2)?,
                    content: row.get(3)?,
                    source: row.get(4)?,
                    in_this_conversation: row.get(5)?,
                    conversation_id: row.get(6)?,
                    sequence: row.get(7)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(hits)
    }

    /// The turns `from..=to` of one conversation, in order — what the
    /// read_conversation tool hands the model once the drill has found a
    /// hit. Same wall as the drill, failing closed: a conversation that is
    /// not the asking companion's reads as None, never as someone else's
    /// words. Same live-turn rule too: from the thread being drilled, the
    /// latest user message and after it stay out.
    pub(crate) fn read_turns(
        &self,
        conversation_id: &str,
        companion_id: &str,
        current_conversation_id: Option<&str>,
        from: i64,
        to: i64,
    ) -> Result<Option<ArchiveWindow>, AppError> {
        let connection = self.connection()?;
        // A thread the companion took part in is one it may read back — its
        // own rows say so, whatever the thread's label says now. A thread it
        // never spoke in stays closed even if the picker points there today.
        let header: Option<(String, Option<String>, i64)> = connection
            .query_row(
                "SELECT c.title, c.source,
                        (SELECT COALESCE(MAX(sequence), 0) FROM messages WHERE conversation_id = c.id)
                 FROM conversations c
                 WHERE c.id = ?1
                   AND EXISTS (SELECT 1 FROM messages m
                               WHERE m.conversation_id = c.id
                                 AND COALESCE(m.companion_id, c.companion_id) = ?2)",
                params![conversation_id, companion_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(AppError::database)?;
        let Some((conversation_title, source, last_sequence)) = header else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT m.sequence, m.role,
                        date(m.created_at / 1000, 'unixepoch') AS day,
                        m.content,
                        (SELECT COUNT(*) FROM message_attachments a WHERE a.message_id = m.id)
                 FROM messages m
                 WHERE m.conversation_id = ?1
                   AND m.status = 'completed'
                   AND m.role IN ('user', 'assistant')
                   AND m.sequence BETWEEN ?2 AND ?3
                   AND (?4 IS NULL
                        OR m.conversation_id <> ?4
                        OR m.sequence < (SELECT COALESCE(MAX(sequence), 0)
                                         FROM messages
                                         WHERE conversation_id = ?4 AND role = 'user'))
                 ORDER BY m.sequence ASC",
            )
            .map_err(AppError::database)?;
        let turns = statement
            .query_map(params![conversation_id, from, to, current_conversation_id], |row| {
                Ok(ArchiveTurn {
                    sequence: row.get(0)?,
                    role: row.get(1)?,
                    day: row.get(2)?,
                    content: row.get(3)?,
                    image_count: row.get(4)?,
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(Some(ArchiveWindow {
            conversation_title,
            source,
            last_sequence,
            turns,
        }))
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
                            provider_id, model_id, companion_id, error_message,
                            created_at, updated_at, completed_at, slept_at
                         ) VALUES (?1, ?2, ?3, ?4, 'completed', ?5,
                                   NULL, NULL, ?7, NULL, ?6, ?6, ?6, ?6)",
                        params![
                            format!("{}#{sequence}", record.id),
                            record.id,
                            sequence as i64,
                            turn.role,
                            turn.text,
                            record.created_at,
                            record.companion_id,
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
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS}
                 FROM messages
                 WHERE conversation_id = ?1
                 ORDER BY sequence ASC"
            ))
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
                    provider_id, model_id, companion_id, error_message,
                    created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?6, 'completed', ?4, NULL, NULL, ?7, NULL, ?5, ?5, ?5)",
                params![
                    message_id,
                    conversation.id,
                    sequence,
                    content,
                    timestamp,
                    role,
                    companion_id
                ],
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
                companion_id: Some(companion_id.to_owned()),
                error_message: None,
                created_at: timestamp,
                updated_at: timestamp,
                completed_at: Some(timestamp),
                slept_at: None,
                attachments: attachments.to_vec(),
            },
        })
    }

    /// Who a live conversation answers to, and whether anyone has spoken in
    /// it yet — the two facts the companion lock is decided on (s569). Cheap
    /// on purpose: the lock is asked on every send, and loading the whole
    /// thread to read one column was the old cost.
    pub(crate) fn ownership(
        &self,
        conversation_id: &str,
    ) -> Result<Option<Ownership>, AppError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT companion_id,
                        EXISTS(
                            SELECT 1 FROM messages
                            WHERE messages.conversation_id = conversations.id
                        )
                 FROM conversations
                 WHERE conversations.id = ?1 AND archived_at IS NULL",
                [conversation_id],
                |row| {
                    Ok(Ownership {
                        companion_id: row.get(0)?,
                        spoken: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(AppError::database)
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

    /// The titler's write: the thread's name, once a model has read enough
    /// of it to give a real one. Not activity — `updated_at` stays put, so a
    /// rename never reorders the sidebar.
    pub(crate) fn rename_conversation(
        &self,
        conversation_id: &str,
        title: &str,
    ) -> Result<Conversation, AppError> {
        let connection = self.connection()?;
        let changed = connection
            .execute(
                "UPDATE conversations SET title = ?2 WHERE id = ?1 AND archived_at IS NULL",
                params![conversation_id, title],
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
        companion_id: &str,
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
                    provider_id, model_id, companion_id, error_message,
                    created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, 'assistant', 'streaming', '', ?4, ?5, ?7, NULL, ?6, ?6, NULL)",
                params![
                    message_id,
                    conversation_id,
                    sequence,
                    provider_id,
                    model_id,
                    timestamp,
                    companion_id
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
            companion_id: Some(companion_id.to_owned()),
            error_message: None,
            created_at: timestamp,
            updated_at: timestamp,
            completed_at: None,
            slept_at: None,
            attachments: Vec::new(),
        })
    }

    /// The provider named the model that is actually answering this row —
    /// overwrite the one the request asked for. Streaming rows only: a
    /// closed row's record is settled.
    pub(crate) fn record_served_model(
        &self,
        message_id: &str,
        model_id: &str,
    ) -> Result<Message, AppError> {
        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE messages
                 SET model_id = ?2
                 WHERE id = ?1 AND role = 'assistant' AND status = 'streaming'",
                params![message_id, model_id],
            )
            .map_err(AppError::database)?;
        if updated != 1 {
            return Err(AppError::internal(
                "the assistant message was not in a streamable state",
            ));
        }
        message_by_id(&connection, message_id)?
            .ok_or_else(|| AppError::internal("the served assistant message could not be reloaded"))
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
        companion_id: row.get(13)?,
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
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
            [message_id],
            message_from_row,
        )
        .optional()
        .map_err(AppError::database)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ChatRepository, CommitUserMessage, MessageAttachment};
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
                companion_id,
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

    /// The drill reaches the OTHER conversations and the earlier turns of
    /// THIS one alike — only the live turn (the latest user message and what
    /// followed it) stays out. s559: a companion drilling from inside the
    /// very thread it was looking for used to be told nothing matched.
    #[test]
    fn the_raw_memory_drill_finds_ranks_and_reaches_its_own_earlier_turns() {
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
            "The Long Serpent again, earlier in the conversation being drilled from.",
            "Understood, the Serpent.",
        );
        // The live turn: the question that triggered the drill. The model is
        // holding it — it is the one thing the drill must not echo back.
        repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: Some(&current),
                companion_id: &companion_id,
                content: "Do you remember the Serpent?",
                title: "ignored",
                timestamp: 1_755_800_010_000,
                new_conversation_id: "unused",
                message_id: "message-current-live",
                attachments: &[],
            })
            .expect("the live turn should commit");

        let hits = repository
            .search_messages("\"serpent\"", &companion_id, Some(&current), 10)
            .expect("search should succeed");
        let contents: Vec<&str> = hits.iter().map(|hit| hit.content.as_str()).collect();
        assert_eq!(hits.len(), 4, "both conversations, minus the live turn: {contents:?}");
        assert!(
            !contents.contains(&"Do you remember the Serpent?"),
            "the live turn never comes back: {contents:?}"
        );
        assert!(
            contents.contains(&"The Long Serpent again, earlier in the conversation being drilled from."),
            "the current thread's EARLIER turns are reachable: {contents:?}"
        );
        assert!(
            contents.contains(&"My favorite ship is the Long Serpent."),
            "the WHOLE message comes back, not a snippet: {contents:?}"
        );
        for hit in &hits {
            assert_eq!(
                hit.in_this_conversation,
                hit.conversation_title == "Talk about current",
                "a hit knows whether it sits in the drilling thread: {}",
                hit.content
            );
        }
        assert_eq!(hits[0].day, "2025-08-21");
        let roles: Vec<&str> = hits.iter().map(|hit| hit.role.as_str()).collect();
        assert!(roles.contains(&"user") && roles.contains(&"assistant"));

        // A message still streaming (never completed) must be invisible.
        repository
            .begin_assistant_message(&ships, "message-ships-streaming", &companion_id, "test", "test-model", 1_755_800_003_000)
            .expect("streaming message should begin");
        let hits = repository
            .search_messages("\"serpent\"", &companion_id, None, 10)
            .expect("search should succeed");
        assert_eq!(hits.len(), 5, "no current thread named: everything completed, nothing live");
        assert!(hits.iter().all(|hit| !hit.in_this_conversation));

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// A second companion on the roster, the way the app makes one: its own
    /// memory agent, not built-in. Inserted raw because the chat repository
    /// has no business creating companions.
    fn companion(path: &std::path::Path, name: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        rusqlite::Connection::open(path)
            .expect("test database should open")
            .execute(
                "INSERT INTO companions (
                    id, name, memory_agent_name, is_built_in,
                    model_preference_mode, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 0, 'inherit', 1, 1)",
                rusqlite::params![id, name, format!("agent-{id}")],
            )
            .expect("the companion should insert");
        id
    }

    /// The reader hands back a stretch of one conversation in order, counts
    /// the images a turn carried, keeps the live turn out, and reads another
    /// companion's thread as nothing at all. s559: the drill found the turn,
    /// the companion had no way to read what was said around it.
    #[test]
    fn the_reader_returns_a_window_of_turns_behind_the_same_wall() {
        let (repository, path) = open_repository("reader");
        let rook = built_in_id(&path);
        let hugin = companion(&path, "Hugin");
        let thread = seed_conversation(
            &repository,
            &hugin,
            "avatar",
            "No, it's not you — I want each AI to choose its own visual.",
            "Oh, that's a thoughtful feature!",
        );
        let picture = MessageAttachment {
            id: "attachment-1".to_owned(),
            media_type: "image/png".to_owned(),
            data: "aGk=".to_owned(),
        };
        repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: Some(&thread),
                companion_id: &hugin,
                content: "Is this close to what you had in mind?",
                title: "ignored",
                timestamp: 1_755_800_010_000,
                new_conversation_id: "unused",
                message_id: "message-avatar-shown",
                attachments: std::slice::from_ref(&picture),
            })
            .expect("the picture turn should commit");
        repository
            .begin_assistant_message(&thread, "message-avatar-yes", &hugin, "test", "test-model", 1_755_800_011_000)
            .expect("assistant message should begin");
        repository
            .complete_assistant_message("message-avatar-yes", "Oh wow, yes! That's incredibly close.", 1_755_800_012_000)
            .expect("assistant message should complete");
        repository
            .commit_user_message(CommitUserMessage {
                role: "user",
                conversation_id: Some(&thread),
                companion_id: &hugin,
                content: "Do you remember when we made your avatar?",
                title: "ignored",
                timestamp: 1_755_800_020_000,
                new_conversation_id: "unused",
                message_id: "message-avatar-live",
                attachments: &[],
            })
            .expect("the live turn should commit");

        // From another thread: the whole stretch, images counted.
        let window = repository
            .read_turns(&thread, &hugin, None, 1, 3)
            .expect("read should succeed")
            .expect("Hugin's own thread reads");
        assert_eq!(window.conversation_title, "Talk about avatar");
        assert_eq!(window.last_sequence, 4);
        let seen: Vec<(i64, &str, i64)> = window
            .turns
            .iter()
            .map(|turn| (turn.sequence, turn.content.as_str(), turn.image_count))
            .collect();
        assert_eq!(
            seen,
            vec![
                (1, "Oh, that's a thoughtful feature!", 0),
                (2, "Is this close to what you had in mind?", 1),
                (3, "Oh wow, yes! That's incredibly close.", 0),
            ]
        );

        // From inside the thread: the same window minus the live turn.
        let inside = repository
            .read_turns(&thread, &hugin, Some(&thread), 0, 10)
            .expect("read should succeed")
            .expect("Hugin's own thread reads");
        assert_eq!(inside.turns.len(), 4, "turns 0-3; the live turn 4 stays out");
        assert!(inside.turns.iter().all(|turn| turn.sequence < 4));

        // Rook asking for Hugin's thread gets nothing — not an error, not a word.
        assert!(repository
            .read_turns(&thread, &rook, None, 0, 10)
            .expect("read should succeed")
            .is_none());

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// The wall between companions, drawn around the RAW memory too: a
    /// companion's drill reaches only the conversations it was part of.
    /// s541 — Rook could search Hugin's past, word for word.
    #[test]
    fn the_drill_never_crosses_into_another_companions_past() {
        let (repository, path) = open_repository("drill-walls");
        let rook = built_in_id(&path);
        let hugin = companion(&path, "Hugin");
        seed_conversation(
            &repository,
            &rook,
            "rook-ships",
            "Rook, my favorite ship is the Long Serpent.",
            "The Long Serpent — Olaf's flagship, sixty oars.",
        );
        seed_conversation(
            &repository,
            &hugin,
            "hugin-ships",
            "Hugin, between us: the Serpent's keel was rotten.",
            "I will keep the Serpent's secret.",
        );

        let rook_hits = repository
            .search_messages("\"serpent\"", &rook, None, 10)
            .expect("search should succeed");
        assert_eq!(rook_hits.len(), 2, "Rook sees exactly his own two turns");
        assert!(
            rook_hits.iter().all(|hit| hit.conversation_title == "Talk about rook-ships"),
            "not one of Hugin's turns leaks into Rook's drill: {:?}",
            rook_hits.iter().map(|hit| &hit.content).collect::<Vec<_>>()
        );

        let hugin_hits = repository
            .search_messages("\"serpent\"", &hugin, None, 10)
            .expect("search should succeed");
        assert_eq!(hugin_hits.len(), 2, "Hugin sees exactly his own two turns");
        assert!(hugin_hits
            .iter()
            .all(|hit| hit.conversation_title == "Talk about hugin-ships"));

        // A companion nobody has talked to has nothing to find — not
        // everyone's past, nothing.
        let stranger = companion(&path, "Stranger");
        let stranger_hits = repository
            .search_messages("\"serpent\"", &stranger, None, 10)
            .expect("search should succeed");
        assert!(stranger_hits.is_empty(), "a fresh companion drills an empty past");

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// The wall is drawn around the ROW, not the thread (s564). A thread's
    /// companion is a label; the rows keep who actually spoke. Re-pointing
    /// Rook's thread at Hugin must not hand Hugin Rook's words — and must not
    /// take them from Rook. Since s569 the SERVICE refuses this re-point on a
    /// spoken thread; the repository stays permissive so the row wall is
    /// proven on its own, for the threads that predate the lock.
    #[test]
    fn re_pointing_a_thread_does_not_re_attribute_what_was_already_said() {
        let (repository, path) = open_repository("row-attribution");
        let rook = built_in_id(&path);
        let hugin = companion(&path, "Hugin");
        let thread = seed_conversation(
            &repository,
            &rook,
            "rook-serpent",
            "Rook, my favorite ship is the Long Serpent.",
            "The Long Serpent — Olaf's flagship, sixty oars.",
        );

        // The picker flips the thread to Hugin. Label changes; rows do not.
        repository
            .update_companion(&thread, &hugin)
            .expect("the thread should re-point");
        let relabelled = repository
            .get_thread(&thread)
            .expect("thread should reload")
            .expect("thread should exist");
        assert_eq!(relabelled.conversation.companion_id.as_deref(), Some(hugin.as_str()));
        assert!(
            relabelled
                .messages
                .iter()
                .all(|message| message.companion_id.as_deref() == Some(rook.as_str())),
            "every row still belongs to the companion it was said by or to"
        );

        // Hugin drills for the Serpent and finds nothing: he was not there.
        assert!(
            repository
                .search_messages("\"serpent\"", &hugin, None, 10)
                .expect("search should succeed")
                .is_empty(),
            "a label change is not a memory transfer"
        );
        assert!(repository
            .read_turns(&thread, &hugin, None, 0, 10)
            .expect("read should succeed")
            .is_none());

        // Rook still owns what he said, whatever the label says now.
        let rook_hits = repository
            .search_messages("\"serpent\"", &rook, None, 10)
            .expect("search should succeed");
        assert_eq!(rook_hits.len(), 2);
        let window = repository
            .read_turns(&thread, &rook, None, 0, 10)
            .expect("read should succeed")
            .expect("Rook can read back the thread he spoke in");
        assert_eq!(window.turns.len(), 2);

        // Hugin's first answer on the re-pointed thread is HIS row, on a thread
        // that now holds both speakers — each row telling the truth about itself.
        repository
            .begin_assistant_message(&thread, "message-hugin-first", &hugin, "test", "test-model", 1_755_800_005_000)
            .expect("assistant message should begin");
        repository
            .complete_assistant_message("message-hugin-first", "I hear the Serpent had a rotten keel.", 1_755_800_006_000)
            .expect("assistant message should complete");
        let mixed = repository
            .get_thread(&thread)
            .expect("thread should reload")
            .expect("thread should exist");
        let speakers: Vec<Option<&str>> = mixed
            .messages
            .iter()
            .map(|message| message.companion_id.as_deref())
            .collect();
        assert_eq!(speakers, vec![Some(rook.as_str()), Some(rook.as_str()), Some(hugin.as_str())]);
        let hugin_hits = repository
            .search_messages("\"serpent\"", &hugin, None, 10)
            .expect("search should succeed");
        assert_eq!(hugin_hits.len(), 1, "Hugin finds his own word, not Rook's two");

        drop(repository);
        let _ = fs::remove_file(path);
    }

    /// The provider's word on which model answered overwrites the request's
    /// guess — on a streaming row only; a settled row is settled.
    #[test]
    fn the_served_model_corrects_the_row_while_it_streams() {
        let (repository, path) = open_repository("served-model");
        let companion_id = built_in_id(&path);
        let thread = seed_conversation(
            &repository,
            &companion_id,
            "served",
            "Which model are you, really?",
            "The one you asked for, I hope.",
        );
        repository
            .begin_assistant_message(&thread, "message-served", &companion_id, "openrouter", "qwen/qwen3.8-27b", 1_755_800_005_000)
            .expect("assistant message should begin");
        let served = repository
            .record_served_model("message-served", "deepseek-ai/DeepSeek-V4-Flash-0731")
            .expect("the served model should record");
        assert_eq!(served.model_id.as_deref(), Some("deepseek-ai/DeepSeek-V4-Flash-0731"));
        assert_eq!(served.provider_id.as_deref(), Some("openrouter"), "the provider is unchanged");
        repository
            .complete_assistant_message("message-served", "Not the one you asked for.", 1_755_800_006_000)
            .expect("assistant message should complete");
        assert!(
            repository
                .record_served_model("message-served", "something-else")
                .is_err(),
            "a completed row's record is settled"
        );

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
            .begin_assistant_message(&conversation, "message-ripe-open", &companion_id, "test", "test-model", 1_755_800_003_000)
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
            .search_messages("\"dmt\"", &companion_id, None, 10)
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
            .search_messages("\"dmt\"", &companion_id, None, 10)
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
            .search_messages("\"dmt\"", &companion_id, None, 10)
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
