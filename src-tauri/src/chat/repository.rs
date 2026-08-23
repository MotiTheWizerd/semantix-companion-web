use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{AcceptedMessage, Conversation, ConversationThread, Message};
use crate::{app_error::AppError, database, preferences::ModelPreference};

pub(crate) struct ChatRepository {
    connection: Mutex<Connection>,
}

pub(crate) struct CommitUserMessage<'a> {
    pub(crate) conversation_id: Option<&'a str>,
    pub(crate) model_preference: &'a ModelPreference,
    pub(crate) selected_model_id: Option<&'a str>,
    pub(crate) content: &'a str,
    pub(crate) title: &'a str,
    pub(crate) timestamp: i64,
    pub(crate) new_conversation_id: &'a str,
    pub(crate) message_id: &'a str,
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
                        m.content
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
                })
            })
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;
        Ok(hits)
    }

    pub(crate) fn list_conversations(&self) -> Result<Vec<Conversation>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, title, model_preference_mode, selected_model_id,
                        created_at, updated_at, archived_at
                 FROM conversations
                 WHERE archived_at IS NULL
                 ORDER BY updated_at DESC, created_at DESC",
            )
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
                "SELECT id, title, model_preference_mode, selected_model_id,
                        created_at, updated_at, archived_at
                 FROM conversations
                 WHERE id = ?1",
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
        let messages = statement
            .query_map([conversation_id], message_from_row)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

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
            model_preference,
            selected_model_id,
            content,
            title,
            timestamp,
            new_conversation_id,
            message_id,
        } = input;
        let (model_preference_mode, _) = model_preference.storage_parts();
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;

        let conversation = if let Some(conversation_id) = conversation_id {
            let mut conversation = transaction
                .query_row(
                    "SELECT id, title, model_preference_mode, selected_model_id,
                            created_at, updated_at, archived_at
                     FROM conversations
                     WHERE id = ?1 AND archived_at IS NULL",
                    [conversation_id],
                    conversation_from_row,
                )
                .optional()
                .map_err(AppError::database)?
                .ok_or_else(|| AppError::validation("That conversation no longer exists."))?;
            transaction
                .execute(
                    "UPDATE conversations
                     SET selected_model_id = ?2,
                         model_preference_mode = ?3
                     WHERE id = ?1",
                    params![conversation_id, selected_model_id, model_preference_mode],
                )
                .map_err(AppError::database)?;
            conversation.model_preference = model_preference.clone();
            conversation
        } else {
            transaction
                .execute(
                    "INSERT INTO conversations (
                        id, title, selected_model_id, created_at, updated_at, archived_at,
                        model_preference_mode
                     ) VALUES (?1, ?2, ?3, ?4, ?4, NULL, ?5)",
                    params![
                        new_conversation_id,
                        title,
                        selected_model_id,
                        timestamp,
                        model_preference_mode
                    ],
                )
                .map_err(AppError::database)?;

            Conversation {
                id: new_conversation_id.to_owned(),
                title: title.to_owned(),
                model_preference: model_preference.clone(),
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
                 ) VALUES (?1, ?2, ?3, 'user', 'completed', ?4, NULL, NULL, NULL, ?5, ?5, ?5)",
                params![message_id, conversation.id, sequence, content, timestamp],
            )
            .map_err(AppError::database)?;
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
                role: "user".to_owned(),
                status: "completed".to_owned(),
                content: content.to_owned(),
                provider_id: None,
                model_id: None,
                error_message: None,
                created_at: timestamp,
                updated_at: timestamp,
                completed_at: Some(timestamp),
                slept_at: None,
            },
        })
    }

    pub(crate) fn update_model_preference(
        &self,
        conversation_id: &str,
        model_preference: &ModelPreference,
    ) -> Result<Conversation, AppError> {
        let (mode, selected_model_id) = model_preference.storage_parts();
        let connection = self.connection()?;
        let changed = connection
            .execute(
                "UPDATE conversations
                 SET model_preference_mode = ?2,
                     selected_model_id = ?3
                 WHERE id = ?1 AND archived_at IS NULL",
                params![conversation_id, mode, selected_model_id],
            )
            .map_err(AppError::database)?;
        if changed == 0 {
            return Err(AppError::validation("That conversation no longer exists."));
        }
        connection
            .query_row(
                "SELECT id, title, model_preference_mode, selected_model_id,
                        created_at, updated_at, archived_at
                 FROM conversations
                 WHERE id = ?1",
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
    let mode: String = row.get(2)?;
    let model_id: Option<String> = row.get(3)?;
    Ok(Conversation {
        id: row.get(0)?,
        title: row.get(1)?,
        model_preference: ModelPreference::from_storage(&mode, model_id),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        archived_at: row.get(6)?,
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
    })
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
    use crate::{database, preferences::ModelPreference};

    fn open_repository(tag: &str) -> (ChatRepository, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "companion-archive-test-{tag}-{}.db",
            uuid::Uuid::new_v4()
        ));
        database::initialise(&path).expect("test database should initialise");
        (ChatRepository::open(&path).expect("repository should open"), path)
    }

    fn seed_conversation(
        repository: &ChatRepository,
        tag: &str,
        user_text: &str,
        assistant_text: &str,
    ) -> String {
        let conversation_id = format!("conversation-{tag}");
        repository
            .commit_user_message(CommitUserMessage {
                conversation_id: None,
                model_preference: &ModelPreference::Test,
                selected_model_id: None,
                content: user_text,
                title: format!("Talk about {tag}").as_str(),
                timestamp: 1_755_800_000_000,
                new_conversation_id: &conversation_id,
                message_id: &format!("message-{tag}-user"),
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
        let ships = seed_conversation(
            &repository,
            "ships",
            "My favorite ship is the Long Serpent.",
            "The Long Serpent was Olaf Tryggvason's flagship.",
        );
        let current = seed_conversation(
            &repository,
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
}
