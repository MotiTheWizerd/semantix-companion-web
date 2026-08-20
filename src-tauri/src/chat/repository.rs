use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{AcceptedMessage, Conversation, ConversationThread, Message};
use crate::{app_error::AppError, database};

pub(crate) struct ChatRepository {
    connection: Mutex<Connection>,
}

impl ChatRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    pub(crate) fn list_conversations(&self) -> Result<Vec<Conversation>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT id, title, selected_model_id, created_at, updated_at, archived_at
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

    pub(crate) fn get_thread(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationThread>, AppError> {
        let connection = self.connection()?;
        let conversation = connection
            .query_row(
                "SELECT id, title, selected_model_id, created_at, updated_at, archived_at
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
                        provider_id, model_id, error_message, created_at, updated_at, completed_at
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
        conversation_id: Option<&str>,
        content: &str,
        title: &str,
        timestamp: i64,
        new_conversation_id: &str,
        message_id: &str,
    ) -> Result<AcceptedMessage, AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;

        let conversation = if let Some(conversation_id) = conversation_id {
            transaction
                .query_row(
                    "SELECT id, title, selected_model_id, created_at, updated_at, archived_at
                     FROM conversations
                     WHERE id = ?1 AND archived_at IS NULL",
                    [conversation_id],
                    conversation_from_row,
                )
                .optional()
                .map_err(AppError::database)?
                .ok_or_else(|| AppError::validation("That conversation no longer exists."))?
        } else {
            transaction
                .execute(
                    "INSERT INTO conversations (
                        id, title, selected_model_id, created_at, updated_at, archived_at
                     ) VALUES (?1, ?2, NULL, ?3, ?3, NULL)",
                    params![new_conversation_id, title, timestamp],
                )
                .map_err(AppError::database)?;

            Conversation {
                id: new_conversation_id.to_owned(),
                title: title.to_owned(),
                selected_model_id: None,
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
            },
        })
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
        selected_model_id: row.get(2)?,
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
    })
}
