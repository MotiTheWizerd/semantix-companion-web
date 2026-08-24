use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::{AgentMessage, SendAgentMessage};
use crate::{app_error::AppError, credentials::unix_timestamp_ms, database};

const MESSAGE_COLUMNS: &str = "id, from_agent_id, to_agent_id, from_user_id, to_user_id,
     project_id, body, created_at, read_at";

pub(crate) struct AgentMailRepository {
    connection: Mutex<Connection>,
}

impl AgentMailRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    /// Send. The id and timestamp are minted here — never accepted from the
    /// caller — so the order of a correspondence cannot be forged.
    ///
    /// No check that the recipient exists, and that is deliberate: an id may
    /// name a companion on another machine. Delivery is not the same question
    /// as addressing, and conflating them would cap the roster at whoever
    /// happens to be local.
    pub(crate) fn send(&self, message: SendAgentMessage<'_>) -> Result<AgentMessage, AppError> {
        let body = message.validate()?;
        let record = AgentMessage {
            id: Uuid::new_v4().to_string(),
            from_agent_id: message.from_agent_id.trim().to_owned(),
            to_agent_id: message.to_agent_id.trim().to_owned(),
            from_user_id: message.from_user_id.map(str::to_owned),
            to_user_id: message.to_user_id.map(str::to_owned),
            project_id: message.project_id.map(str::to_owned),
            body,
            created_at: unix_timestamp_ms()?,
            read_at: None,
        };

        self.connection()?
            .execute(
                "INSERT INTO agent_messages (
                    id, from_agent_id, to_agent_id, from_user_id, to_user_id,
                    project_id, body, created_at, read_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    record.id,
                    record.from_agent_id,
                    record.to_agent_id,
                    record.from_user_id,
                    record.to_user_id,
                    record.project_id,
                    record.body,
                    record.created_at,
                ],
            )
            .map_err(AppError::database)?;

        Ok(record)
    }

    /// Everything addressed to one agent, newest first.
    ///
    /// ⚑ THE PRIVACY LIVES IN THIS SIGNATURE. There is no variant that takes a
    /// third party's id and no filter that widens the scope — the only mail a
    /// caller can name is its own. Keep it that way: the moment a general
    /// `query(...)` exists, "nobody reads another's mail" becomes a convention
    /// instead of a property.
    pub(crate) fn inbox(&self, agent_id: &str, unread_only: bool) -> Result<Vec<AgentMessage>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS}
                 FROM agent_messages
                 WHERE to_agent_id = ?1 AND (?2 = 0 OR read_at IS NULL)
                 ORDER BY created_at DESC, id ASC"
            ))
            .map_err(AppError::database)?;

        let messages = statement
            .query_map(params![agent_id, i64::from(unread_only)], map_message)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(messages)
    }

    /// Everything one agent has sent, newest first. The other half of "own mail
    /// only" — a sender may see its own record of what it wrote.
    ///
    /// Along with `unread_count` and `get` below: tested, and waiting on the
    /// inbox UI, which is what a human needs them for. The model's four tools
    /// do not — an agent reads its inbox and marks things read; a person wants
    /// a badge, a sent folder, and one message open.
    #[allow(dead_code)]
    pub(crate) fn sent(&self, agent_id: &str) -> Result<Vec<AgentMessage>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS}
                 FROM agent_messages
                 WHERE from_agent_id = ?1
                 ORDER BY created_at DESC, id ASC"
            ))
            .map_err(AppError::database)?;

        let messages = statement
            .query_map([agent_id], map_message)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(messages)
    }

    /// The badge. One indexed count rather than loading an inbox to measure it.
    #[allow(dead_code)]
    pub(crate) fn unread_count(&self, agent_id: &str) -> Result<i64, AppError> {
        self.connection()?
            .query_row(
                "SELECT COUNT(*) FROM agent_messages
                 WHERE to_agent_id = ?1 AND read_at IS NULL",
                [agent_id],
                |row| row.get(0),
            )
            .map_err(AppError::database)
    }

    /// Mark one letter read. Scoped to the RECIPIENT: the id alone is not
    /// authority, or knowing a uuid would let anyone clear anyone's inbox.
    /// Already-read mail keeps its original timestamp — read_at records when it
    /// was first seen, and a second call must not rewrite that.
    pub(crate) fn mark_read(&self, agent_id: &str, message_id: &str) -> Result<bool, AppError> {
        let now = unix_timestamp_ms()?;
        let changed = self
            .connection()?
            .execute(
                "UPDATE agent_messages
                 SET read_at = ?3
                 WHERE id = ?1 AND to_agent_id = ?2 AND read_at IS NULL",
                params![message_id, agent_id, now],
            )
            .map_err(AppError::database)?;
        Ok(changed > 0)
    }

    /// One letter, readable only by the two ends that hold it.
    #[allow(dead_code)]
    pub(crate) fn get(&self, agent_id: &str, message_id: &str) -> Result<Option<AgentMessage>, AppError> {
        self.connection()?
            .query_row(
                &format!(
                    "SELECT {MESSAGE_COLUMNS} FROM agent_messages
                     WHERE id = ?1 AND (to_agent_id = ?2 OR from_agent_id = ?2)"
                ),
                params![message_id, agent_id],
                map_message,
            )
            .optional()
            .map_err(AppError::database)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the mail database lock was poisoned"))
    }
}

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentMessage> {
    Ok(AgentMessage {
        id: row.get(0)?,
        from_agent_id: row.get(1)?,
        to_agent_id: row.get(2)?,
        from_user_id: row.get(3)?,
        to_user_id: row.get(4)?,
        project_id: row.get(5)?,
        body: row.get(6)?,
        created_at: row.get(7)?,
        read_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn open_mail(tag: &str) -> (AgentMailRepository, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "companion-mail-test-{tag}-{}.db",
            Uuid::new_v4()
        ));
        database::initialise(&path).expect("test database should initialise");
        (
            AgentMailRepository::open(&path).expect("mail repository should open"),
            path,
        )
    }

    fn letter<'a>(from: &'a str, to: &'a str, body: &'a str) -> SendAgentMessage<'a> {
        SendAgentMessage {
            from_agent_id: from,
            to_agent_id: to,
            from_user_id: None,
            to_user_id: None,
            project_id: None,
            body,
        }
    }

    #[test]
    fn a_letter_arrives_unread_and_only_in_the_recipients_inbox() {
        let (mail, path) = open_mail("arrives");

        let sent = mail
            .send(letter("huginn", "rook", "  What should we call each other?  "))
            .expect("the letter should send");
        assert!(sent.read_at.is_none(), "new mail is unread");
        assert_eq!(sent.body, "What should we call each other?", "body is trimmed");

        let rooks = mail.inbox("rook", false).expect("inbox should read");
        assert_eq!(rooks.len(), 1);
        assert_eq!(rooks[0], sent);
        assert_eq!(mail.unread_count("rook").unwrap(), 1);

        // The sender's own inbox stays empty — mail goes one way.
        assert!(mail.inbox("huginn", false).unwrap().is_empty());
        assert_eq!(mail.unread_count("huginn").unwrap(), 0);
        assert_eq!(mail.sent("huginn").unwrap().len(), 1);

        fs::remove_file(path).ok();
    }

    #[test]
    fn nobody_can_read_or_clear_another_agents_mail() {
        let (mail, path) = open_mail("privacy");
        let sent = mail
            .send(letter("huginn", "rook", "Between us two."))
            .expect("the letter should send");

        // A third agent sees nothing, by any of the three read paths.
        assert!(mail.inbox("magpie", false).unwrap().is_empty());
        assert!(mail.sent("magpie").unwrap().is_empty());
        assert_eq!(mail.unread_count("magpie").unwrap(), 0);
        assert!(
            mail.get("magpie", &sent.id).unwrap().is_none(),
            "knowing the id must not be enough to read the letter"
        );

        // Nor can it clear someone else's unread badge.
        assert!(
            !mail.mark_read("magpie", &sent.id).unwrap(),
            "a stranger must not be able to mark mail read"
        );
        assert_eq!(mail.unread_count("rook").unwrap(), 1, "still unread");

        // Both real ends can read it.
        assert!(mail.get("rook", &sent.id).unwrap().is_some());
        assert!(mail.get("huginn", &sent.id).unwrap().is_some());

        fs::remove_file(path).ok();
    }

    #[test]
    fn reading_is_recorded_once_and_the_first_time_stands() {
        let (mail, path) = open_mail("read");
        let sent = mail.send(letter("huginn", "rook", "Read me.")).unwrap();

        assert!(mail.mark_read("rook", &sent.id).unwrap());
        let first = mail.get("rook", &sent.id).unwrap().unwrap().read_at;
        assert!(first.is_some());
        assert_eq!(mail.unread_count("rook").unwrap(), 0);
        assert!(mail.inbox("rook", true).unwrap().is_empty(), "unread filter");
        assert_eq!(mail.inbox("rook", false).unwrap().len(), 1, "still in the inbox");

        // A second call changes nothing — read_at is when it was FIRST seen.
        assert!(!mail.mark_read("rook", &sent.id).unwrap());
        assert_eq!(mail.get("rook", &sent.id).unwrap().unwrap().read_at, first);

        fs::remove_file(path).ok();
    }

    #[test]
    fn anyone_may_send_to_anyone_including_an_agent_this_machine_never_heard_of() {
        let (mail, path) = open_mail("open-send");

        // No roster check, no foreign key: an id that exists nowhere locally is
        // still a valid address, because one day it will live elsewhere.
        let sent = mail
            .send(letter(
                "rook",
                "7b038b69-33cd-45ec-8460-4844de63d06f",
                "Hello, stranger.",
            ))
            .expect("an unknown recipient is not an error");
        assert_eq!(
            mail.inbox("7b038b69-33cd-45ec-8460-4844de63d06f", false)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(sent.from_agent_id, "rook");

        fs::remove_file(path).ok();
    }

    #[test]
    fn an_empty_or_oversized_letter_is_refused_with_a_sentence() {
        let (mail, path) = open_mail("validation");

        for body in ["", "   ", "\n\t "] {
            let error = mail
                .send(letter("huginn", "rook", body))
                .expect_err("an empty message must be refused");
            assert!(
                error.to_string().contains("needs something in it"),
                "got: {error}"
            );
        }

        let too_long = "x".repeat(super::super::MAX_BODY_LENGTH + 1);
        assert!(mail.send(letter("huginn", "rook", &too_long)).is_err());

        let error = mail
            .send(letter("  ", "rook", "orphan"))
            .expect_err("a message with no sender must be refused");
        assert!(error.to_string().contains("sender"), "got: {error}");

        assert_eq!(mail.unread_count("rook").unwrap(), 0, "nothing was stored");

        fs::remove_file(path).ok();
    }
}
