use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use super::{
    CallStatus, RavenCall, RavenCallMessage, MAX_BODY_LENGTH, MAX_CALLS_PER_DAY,
    MAX_MESSAGES_PER_CALL,
};
use crate::{app_error::AppError, credentials::unix_timestamp_ms, database};

const CALL_COLUMNS: &str = "id, root_conversation_id, initiator_agent_id, status,
     message_count, created_at, closed_at";

const MESSAGE_COLUMNS: &str = "id, call_id, from_agent_id, to_agent_id, body, created_at";

/// Calls opened by one companion since local midnight.
///
/// ⚑ `'localtime'` IS THE WHOLE POINT — SQLite resolves it against the operating
/// system's timezone, which is the clock of the person whose machine this is.
/// It also gets the day right across a daylight-saving change, which hand-rolled
/// midnight arithmetic reliably does not. `created_at` is milliseconds, hence
/// the divide before `unixepoch`.
const CALLS_TODAY: &str = "SELECT COUNT(*) FROM raven_calls
     WHERE initiator_agent_id = ?1
       AND date(created_at / 1000, 'unixepoch', 'localtime')
           = date('now', 'localtime')";

pub(crate) struct RavenCallRepository {
    connection: Mutex<Connection>,
}

impl RavenCallRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    /// Open a call, or refuse because today's allowance is spent.
    ///
    /// ⚑ THE CAP IS ENFORCED HERE, NOT DESCRIBED IN A PROMPT. A limit the model
    /// is merely told about is a suggestion, and the one thing this limit exists
    /// to survive is a model that has stopped reasoning well.
    ///
    /// The refusal is a SENTENCE, and that is deliberate: it is read by a
    /// language model, which will retry a vague failure and give up on a clear
    /// one. It names the number, the reason, and when it resets, so the caller
    /// can tell its human something true instead of apologising in a loop.
    pub(crate) fn open_call(
        &self,
        initiator_agent_id: &str,
        root_conversation_id: Option<&str>,
    ) -> Result<RavenCall, AppError> {
        let initiator = initiator_agent_id.trim();
        if initiator.is_empty() {
            return Err(AppError::validation(
                "a call needs someone to open it — an agent id, not an empty string",
            ));
        }

        let connection = self.connection()?;
        let opened_today: i64 = connection
            .query_row(CALLS_TODAY, [initiator], |row| row.get(0))
            .map_err(AppError::database)?;

        if opened_today >= MAX_CALLS_PER_DAY {
            return Err(AppError::validation(format!(
                "You have used all {MAX_CALLS_PER_DAY} of today's calls. \
                 The allowance resets at midnight, local time. \
                 Do not try again today — tell whoever asked, instead."
            )));
        }

        let record = RavenCall {
            id: Uuid::new_v4().to_string(),
            root_conversation_id: root_conversation_id.map(str::to_owned),
            initiator_agent_id: initiator.to_owned(),
            status: CallStatus::Open,
            message_count: 0,
            created_at: unix_timestamp_ms()?,
            closed_at: None,
        };

        connection
            .execute(
                "INSERT INTO raven_calls (
                    id, root_conversation_id, initiator_agent_id, status,
                    message_count, created_at, closed_at
                 ) VALUES (?1, ?2, ?3, ?4, 0, ?5, NULL)",
                params![
                    record.id,
                    record.root_conversation_id,
                    record.initiator_agent_id,
                    record.status.as_str(),
                    record.created_at,
                ],
            )
            .map_err(AppError::database)?;

        Ok(record)
    }

    /// Add one turn to a call, or refuse because the call is full or finished.
    ///
    /// The count check and the insert share ONE TRANSACTION. Without it two
    /// concurrent hops both read four, both write, and the call ends up with
    /// six turns in a table whose whole purpose was to make that impossible.
    pub(crate) fn append_message(
        &self,
        call_id: &str,
        from_agent_id: &str,
        to_agent_id: &str,
        body: &str,
    ) -> Result<RavenCallMessage, AppError> {
        let body = validate_body(body)?;
        let from = require_id(from_agent_id, "sender")?;
        let to = require_id(to_agent_id, "recipient")?;

        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;

        let existing: Option<(String, i64)> = transaction
            .query_row(
                "SELECT status, message_count FROM raven_calls WHERE id = ?1",
                [call_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(AppError::database)?;

        let Some((status, message_count)) = existing else {
            return Err(AppError::validation(
                "there is no call with that id — open one before writing into it",
            ));
        };

        if status != CallStatus::Open.as_str() {
            return Err(AppError::validation(
                "that call is closed. Open a new one if there is more to say.",
            ));
        }
        if message_count >= MAX_MESSAGES_PER_CALL {
            return Err(AppError::validation(format!(
                "This call has reached its limit of {MAX_MESSAGES_PER_CALL} messages and is now \
                 closed. Open a new call if the exchange genuinely needs to continue — and \
                 remember that opening one spends part of today's allowance."
            )));
        }

        let record = RavenCallMessage {
            id: Uuid::new_v4().to_string(),
            call_id: call_id.to_owned(),
            from_agent_id: from,
            to_agent_id: to,
            body,
            created_at: unix_timestamp_ms()?,
        };

        transaction
            .execute(
                "INSERT INTO raven_call_messages (
                    id, call_id, from_agent_id, to_agent_id, body, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.id,
                    record.call_id,
                    record.from_agent_id,
                    record.to_agent_id,
                    record.body,
                    record.created_at,
                ],
            )
            .map_err(AppError::database)?;

        // The turn that fills a call also ends it. A call left open at its
        // ceiling would accept nothing and still read as live to every caller
        // and every badge — better to have the row say what is true.
        let now_full = message_count + 1 >= MAX_MESSAGES_PER_CALL;
        transaction
            .execute(
                "UPDATE raven_calls
                 SET message_count = message_count + 1,
                     status        = CASE WHEN ?2 THEN 'closed' ELSE status END,
                     closed_at     = CASE WHEN ?2 THEN ?3 ELSE closed_at END
                 WHERE id = ?1",
                params![call_id, now_full, record.created_at],
            )
            .map_err(AppError::database)?;

        transaction.commit().map_err(AppError::database)?;
        Ok(record)
    }

    /// How many calls this companion has left today. The number a caller needs
    /// to decide whether to start something it cannot finish.
    pub(crate) fn calls_remaining_today(&self, agent_id: &str) -> Result<i64, AppError> {
        let opened: i64 = self
            .connection()?
            .query_row(CALLS_TODAY, [agent_id.trim()], |row| row.get(0))
            .map_err(AppError::database)?;
        Ok((MAX_CALLS_PER_DAY - opened).max(0))
    }

    /// The turns of one call, oldest first — reading order.
    ///
    /// `limit` is REQUIRED rather than optional. This table grows unattended at
    /// machine speed, and an unbounded "give me everything" is the query that
    /// looks harmless for a month and then loads a hundred thousand rows into a
    /// UI. A caller that wants more asks twice.
    pub(crate) fn messages(&self, call_id: &str, limit: i64) -> Result<Vec<RavenCallMessage>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM raven_call_messages
                 WHERE call_id = ?1
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?2"
            ))
            .map_err(AppError::database)?;

        let messages = statement
            .query_map(params![call_id, limit.max(1)], map_message)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(messages)
    }

    /// Every call born out of one human conversation, newest first. This is the
    /// provenance query the whole table exists to make answerable: "what did my
    /// companion say to anyone else while working on THIS."
    #[allow(dead_code)]
    pub(crate) fn calls_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<RavenCall>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {CALL_COLUMNS} FROM raven_calls
                 WHERE root_conversation_id = ?1
                 ORDER BY created_at DESC, id ASC"
            ))
            .map_err(AppError::database)?;

        let calls = statement
            .query_map([conversation_id], map_call)
            .map_err(AppError::database)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::database)?;

        Ok(calls)
    }

    #[allow(dead_code)]
    pub(crate) fn get(&self, call_id: &str) -> Result<Option<RavenCall>, AppError> {
        self.connection()?
            .query_row(
                &format!("SELECT {CALL_COLUMNS} FROM raven_calls WHERE id = ?1"),
                [call_id],
                map_call,
            )
            .optional()
            .map_err(AppError::database)
    }

    /// End a call early. Idempotent: closing a closed call keeps the original
    /// `closed_at`, because that stamp records when the exchange actually
    /// stopped and a second call must not rewrite history.
    pub(crate) fn close(&self, call_id: &str) -> Result<bool, AppError> {
        let now = unix_timestamp_ms()?;
        let changed = self
            .connection()?
            .execute(
                "UPDATE raven_calls
                 SET status = 'closed', closed_at = ?2
                 WHERE id = ?1 AND status = 'open'",
                params![call_id, now],
            )
            .map_err(AppError::database)?;
        Ok(changed > 0)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the raven call database lock was poisoned"))
    }
}

fn require_id(value: &str, label: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation(format!(
            "a call message needs a {label} — an agent id, not an empty string"
        )));
    }
    Ok(trimmed.to_owned())
}

fn validate_body(body: &str) -> Result<String, AppError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(AppError::validation("a call message needs something in it"));
    }
    if trimmed.chars().count() > MAX_BODY_LENGTH {
        return Err(AppError::validation(format!(
            "a call message may be at most {MAX_BODY_LENGTH} characters"
        )));
    }
    Ok(trimmed.to_owned())
}

fn map_call(row: &Row<'_>) -> rusqlite::Result<RavenCall> {
    let status: String = row.get(3)?;
    Ok(RavenCall {
        id: row.get(0)?,
        root_conversation_id: row.get(1)?,
        initiator_agent_id: row.get(2)?,
        status: CallStatus::parse(&status).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("unknown call status {status:?}").into(),
            )
        })?,
        message_count: row.get(4)?,
        created_at: row.get(5)?,
        closed_at: row.get(6)?,
    })
}

fn map_message(row: &Row<'_>) -> rusqlite::Result<RavenCallMessage> {
    Ok(RavenCallMessage {
        id: row.get(0)?,
        call_id: row.get(1)?,
        from_agent_id: row.get(2)?,
        to_agent_id: row.get(3)?,
        body: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn open_calls(tag: &str) -> (RavenCallRepository, std::path::PathBuf) {
        let path = std::env::temp_dir()
            .join(format!("companion-calls-test-{tag}-{}.db", Uuid::new_v4()));
        database::initialise(&path).expect("test database should initialise");
        (
            RavenCallRepository::open(&path).expect("call repository should open"),
            path,
        )
    }

    #[test]
    fn a_call_opens_empty_and_holds_its_turns_in_order() {
        let (calls, path) = open_calls("opens");

        let call = calls
            .open_call("rook", None)
            .expect("the first call of the day should open");
        assert_eq!(call.message_count, 0);
        assert_eq!(call.status, CallStatus::Open);
        assert!(call.closed_at.is_none());

        calls
            .append_message(&call.id, "rook", "hugin", "  Are you there?  ")
            .expect("the opening turn should land");
        let reply = calls
            .append_message(&call.id, "hugin", "rook", "I am.")
            .expect("the reply should land");
        assert_eq!(reply.body, "I am.");

        let turns = calls.messages(&call.id, 50).expect("turns should read");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].body, "Are you there?", "body is trimmed");
        assert_eq!(turns[1].from_agent_id, "hugin", "oldest first");

        let reloaded = calls.get(&call.id).unwrap().expect("call should still exist");
        assert_eq!(reloaded.message_count, 2, "the count is kept on the row");

        fs::remove_file(path).ok();
    }

    #[test]
    fn the_sixth_call_of_the_day_is_refused_with_a_sentence_naming_the_reset() {
        let (calls, path) = open_calls("daily-cap");

        for index in 0..MAX_CALLS_PER_DAY {
            assert_eq!(
                calls.calls_remaining_today("rook").unwrap(),
                MAX_CALLS_PER_DAY - index,
                "the allowance counts down as calls are opened"
            );
            calls.open_call("rook", None).expect("within the allowance");
        }
        assert_eq!(calls.calls_remaining_today("rook").unwrap(), 0);

        let refused = calls
            .open_call("rook", None)
            .expect_err("the sixth call must be refused");
        let sentence = refused.to_string();
        assert!(
            sentence.contains(&MAX_CALLS_PER_DAY.to_string()),
            "the refusal names the limit: {sentence}"
        );
        assert!(
            sentence.contains("midnight"),
            "the refusal says when it resets, so the caller stops instead of retrying: {sentence}"
        );

        fs::remove_file(path).ok();
    }

    #[test]
    fn the_daily_cap_is_per_companion_not_per_correspondent() {
        let (calls, path) = open_calls("per-companion");

        // Five calls spread across five different recipients still spend the
        // whole allowance — making friends does not earn more budget.
        for _ in 0..MAX_CALLS_PER_DAY {
            calls.open_call("rook", None).expect("within the allowance");
        }
        assert!(
            calls.open_call("rook", None).is_err(),
            "a sixth call is refused however many agents were addressed"
        );

        // And it is scoped: another companion still has its own full day.
        assert_eq!(
            calls.calls_remaining_today("hugin").unwrap(),
            MAX_CALLS_PER_DAY
        );
        calls
            .open_call("hugin", None)
            .expect("one companion's spending must not drain another's");

        fs::remove_file(path).ok();
    }

    #[test]
    fn a_call_closes_itself_on_its_last_turn_and_then_takes_nothing() {
        let (calls, path) = open_calls("call-cap");
        let call = calls.open_call("rook", None).unwrap();

        for turn in 0..MAX_MESSAGES_PER_CALL {
            calls
                .append_message(&call.id, "rook", "hugin", &format!("turn {turn}"))
                .expect("within the call's limit");
        }

        let full = calls.get(&call.id).unwrap().expect("call should exist");
        assert_eq!(full.message_count, MAX_MESSAGES_PER_CALL);
        assert_eq!(
            full.status,
            CallStatus::Closed,
            "the turn that fills a call also ends it"
        );
        assert!(full.closed_at.is_some(), "a closed call is stamped");

        let refused = calls
            .append_message(&call.id, "hugin", "rook", "one more thing")
            .expect_err("a full call must take nothing further");
        assert!(
            refused.to_string().contains("closed"),
            "the refusal says the call is over: {refused}"
        );

        fs::remove_file(path).ok();
    }

    #[test]
    fn a_reply_spends_the_senders_call_rather_than_opening_its_own() {
        let (calls, path) = open_calls("inherit");

        // ⚑ THE INVARIANT THE WHOLE DESIGN RESTS ON. Hugin answering inside
        // Rook's call must not cost Hugin a call — and must not grant the
        // exchange a second budget. Five turns is five turns, whoever wrote
        // them, and neither side's daily allowance moves.
        let call = calls.open_call("rook", None).unwrap();
        assert_eq!(calls.calls_remaining_today("rook").unwrap(), MAX_CALLS_PER_DAY - 1);

        calls.append_message(&call.id, "rook", "hugin", "a question").unwrap();
        calls.append_message(&call.id, "hugin", "rook", "an answer").unwrap();

        assert_eq!(
            calls.calls_remaining_today("hugin").unwrap(),
            MAX_CALLS_PER_DAY,
            "answering inside someone else's call costs the replier nothing"
        );
        assert_eq!(
            calls.calls_remaining_today("rook").unwrap(),
            MAX_CALLS_PER_DAY - 1,
            "and does not spend a second call for the initiator either"
        );
        assert_eq!(calls.get(&call.id).unwrap().unwrap().message_count, 2);

        fs::remove_file(path).ok();
    }

    #[test]
    fn a_call_belongs_to_the_conversation_it_was_born_from() {
        let (calls, path) = open_calls("provenance");
        let connection = database::open_connection(&path).unwrap();
        let now = unix_timestamp_ms().unwrap();
        connection
            .execute(
                "INSERT INTO conversations (id, title, created_at, updated_at)
                 VALUES ('conv-1', 'the work', ?1, ?1)",
                params![now],
            )
            .expect("a conversation to hang the call from");

        let rooted = calls.open_call("rook", Some("conv-1")).unwrap();
        calls.open_call("rook", None).expect("an unrooted call too");

        let found = calls.calls_for_conversation("conv-1").unwrap();
        assert_eq!(found.len(), 1, "only the rooted call belongs to the thread");
        assert_eq!(found[0].id, rooted.id);

        // ⚑ THE DELETE PATH, PROVEN NOW RATHER THAN WRITTEN LATER UNDER DURESS.
        // Deleting the conversation takes its call, and the call takes its
        // turns. This table grows unattended; a cascade that was never tested
        // is a cascade that does not exist.
        calls
            .append_message(&rooted.id, "rook", "hugin", "about this work")
            .unwrap();
        connection
            .execute("DELETE FROM conversations WHERE id = 'conv-1'", [])
            .expect("the conversation should delete");

        assert!(calls.get(&rooted.id).unwrap().is_none(), "the call went with it");
        assert!(
            calls.messages(&rooted.id, 50).unwrap().is_empty(),
            "and the turns went with the call"
        );

        fs::remove_file(path).ok();
    }

    #[test]
    fn writing_into_a_call_that_does_not_exist_says_so_plainly() {
        let (calls, path) = open_calls("missing");
        let error = calls
            .append_message("no-such-call", "rook", "hugin", "hello?")
            .expect_err("an unknown call id must be refused");
        assert!(
            error.to_string().contains("no call with that id"),
            "{error}"
        );
        fs::remove_file(path).ok();
    }

    #[test]
    fn an_empty_turn_and_an_oversized_one_are_both_refused() {
        let (calls, path) = open_calls("body");
        let call = calls.open_call("rook", None).unwrap();

        assert!(calls.append_message(&call.id, "rook", "hugin", "   ").is_err());
        let huge = "x".repeat(MAX_BODY_LENGTH + 1);
        assert!(calls.append_message(&call.id, "rook", "hugin", &huge).is_err());

        assert_eq!(
            calls.get(&call.id).unwrap().unwrap().message_count,
            0,
            "a refused turn must not have moved the counter"
        );

        fs::remove_file(path).ok();
    }

    #[test]
    fn closing_a_call_twice_keeps_the_first_stamp() {
        let (calls, path) = open_calls("close");
        let call = calls.open_call("rook", None).unwrap();

        assert!(calls.close(&call.id).unwrap(), "the first close takes");
        let closed = calls.get(&call.id).unwrap().unwrap();
        assert_eq!(closed.status, CallStatus::Closed);

        assert!(!calls.close(&call.id).unwrap(), "the second close is a no-op");
        assert_eq!(
            calls.get(&call.id).unwrap().unwrap().closed_at,
            closed.closed_at,
            "closed_at records when the exchange stopped, not when it was last asked about"
        );

        fs::remove_file(path).ok();
    }
}
