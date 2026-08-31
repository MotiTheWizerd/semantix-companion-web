use std::{path::Path, time::Duration};

use rusqlite::Connection;

use crate::app_error::AppError;

const LATEST_SCHEMA_VERSION: i64 = 23;

pub(crate) fn initialise(path: &Path) -> Result<(), AppError> {
    let mut connection = open_connection(path)?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL;")
        .map_err(AppError::database)?;
    migrate(&mut connection)
}

pub(crate) fn open_connection(path: &Path) -> Result<Connection, AppError> {
    let connection = Connection::open(path).map_err(AppError::database)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(AppError::database)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(AppError::database)?;
    Ok(connection)
}

fn migrate(connection: &mut Connection) -> Result<(), AppError> {
    let current_version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(AppError::database)?;

    if current_version > LATEST_SCHEMA_VERSION {
        return Err(AppError::internal(format!(
            "the local database uses schema version {current_version}, but this build supports up to {LATEST_SCHEMA_VERSION}"
        )));
    }
    if current_version == LATEST_SCHEMA_VERSION {
        return Ok(());
    }

    // Enforcement OFF for the duration, then verified before it goes back on.
    //
    // A migration that reshapes a table other tables point at has to drop and
    // recreate it, and with enforcement ON SQLite's DROP TABLE performs an
    // implicit DELETE — which would CASCADE the entire message archive into
    // nothing. This is SQLite's own documented procedure for a table rebuild.
    // The `foreign_key_check` below is the half that must never be skipped:
    // it is what proves the rebuild left every reference intact.
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(AppError::database)?;
    let migrated = apply_migrations(connection, current_version);
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(AppError::database)?;
    migrated?;

    let dangling: i64 = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .map_err(AppError::database)?;
    if dangling > 0 {
        return Err(AppError::internal(format!(
            "the schema upgrade left {dangling} broken references behind"
        )));
    }

    Ok(())
}

fn apply_migrations(connection: &mut Connection, current_version: i64) -> Result<(), AppError> {
    for version in (current_version + 1)..=LATEST_SCHEMA_VERSION {
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute_batch(migration_sql(version))
            .map_err(AppError::database)?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(AppError::database)?;
        transaction.commit().map_err(AppError::database)?;
    }

    Ok(())
}

fn migration_sql(version: i64) -> &'static str {
    match version {
        1 => {
            "CREATE TABLE IF NOT EXISTS provider_credentials (
                 id TEXT PRIMARY KEY,
                 provider_id TEXT NOT NULL,
                 label TEXT NOT NULL,
                 secret_ref TEXT NOT NULL UNIQUE,
                 key_hint TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 last_used_at INTEGER
             );

             CREATE INDEX IF NOT EXISTS idx_provider_credentials_provider
                 ON provider_credentials(provider_id);"
        }
        2 => {
            "CREATE TABLE IF NOT EXISTS configured_models (
                 id TEXT PRIMARY KEY,
                 provider_id TEXT NOT NULL,
                 model_id TEXT NOT NULL,
                 display_name TEXT NOT NULL,
                 credential_id TEXT,
                 secret_ref TEXT UNIQUE,
                 manual_key_hint TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 FOREIGN KEY (credential_id)
                     REFERENCES provider_credentials(id)
                     ON DELETE RESTRICT,
                 CHECK (
                     (credential_id IS NOT NULL AND secret_ref IS NULL AND manual_key_hint IS NULL)
                     OR
                     (credential_id IS NULL AND secret_ref IS NOT NULL AND manual_key_hint IS NOT NULL)
                 )
             );

             CREATE INDEX IF NOT EXISTS idx_configured_models_provider
                 ON configured_models(provider_id);

             CREATE INDEX IF NOT EXISTS idx_configured_models_credential
                 ON configured_models(credential_id);"
        }
        3 => {
            "CREATE TABLE IF NOT EXISTS conversations (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 selected_model_id TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 archived_at INTEGER,
                 FOREIGN KEY (selected_model_id)
                     REFERENCES configured_models(id)
                     ON DELETE SET NULL
             );

             CREATE INDEX IF NOT EXISTS idx_conversations_updated
                 ON conversations(updated_at DESC);

             CREATE TABLE IF NOT EXISTS messages (
                 id TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL,
                 sequence INTEGER NOT NULL CHECK (sequence >= 0),
                 role TEXT NOT NULL CHECK (role IN ('user', 'assistant', 'system', 'tool')),
                 status TEXT NOT NULL CHECK (status IN ('pending', 'streaming', 'completed', 'failed', 'cancelled')),
                 content TEXT NOT NULL DEFAULT '',
                 provider_id TEXT,
                 model_id TEXT,
                 error_message TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 completed_at INTEGER,
                 FOREIGN KEY (conversation_id)
                     REFERENCES conversations(id)
                     ON DELETE CASCADE,
                 UNIQUE (conversation_id, sequence)
             );

             CREATE INDEX IF NOT EXISTS idx_messages_conversation_sequence
                 ON messages(conversation_id, sequence);"
        }
        4 => {
            "ALTER TABLE conversations
                 ADD COLUMN model_preference_mode TEXT NOT NULL DEFAULT 'test'
                 CHECK (model_preference_mode IN ('inherit', 'test', 'configured'));

             UPDATE conversations
                 SET model_preference_mode = CASE
                     WHEN selected_model_id IS NULL THEN 'test'
                     ELSE 'configured'
                 END;

             CREATE TABLE user_preferences (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 default_model_mode TEXT NOT NULL
                     CHECK (default_model_mode IN ('test', 'configured')),
                 default_model_id TEXT,
                 updated_at INTEGER NOT NULL,
                 FOREIGN KEY (default_model_id)
                     REFERENCES configured_models(id)
                     ON DELETE SET NULL
             );

             INSERT INTO user_preferences (
                 id, default_model_mode, default_model_id, updated_at
             ) VALUES (1, 'test', NULL, 0);"
        }
        5 => {
            // Full-text index over message content — the ground under the
            // model's search_conversations tool (its "raw memory" drill).
            // External-content FTS5: the index stores no second copy of the
            // text, and the triggers keep it in lockstep with messages.
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                 content,
                 content='messages',
                 content_rowid='rowid'
             );

             INSERT INTO messages_fts(rowid, content)
                 SELECT rowid, content FROM messages;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_insert
                 AFTER INSERT ON messages BEGIN
                     INSERT INTO messages_fts(rowid, content)
                         VALUES (new.rowid, new.content);
                 END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_delete
                 AFTER DELETE ON messages BEGIN
                     INSERT INTO messages_fts(messages_fts, rowid, content)
                         VALUES ('delete', old.rowid, old.content);
                 END;

             CREATE TRIGGER IF NOT EXISTS messages_fts_after_update
                 AFTER UPDATE OF content ON messages BEGIN
                     INSERT INTO messages_fts(messages_fts, rowid, content)
                         VALUES ('delete', old.rowid, old.content);
                     INSERT INTO messages_fts(rowid, content)
                         VALUES (new.rowid, new.content);
                 END;"
        }
        6 => {
            // The sleep ledger: a message the /sleep pass has already distilled
            // carries the stamp, so a second pass distills only unstamped rows.
            "ALTER TABLE messages ADD COLUMN slept_at INTEGER;"
        }
        7 => {
            // Companions: who you talk to. A companion owns ONE private memory
            // — `memory_agent_name` is its agent on the organ's roster, and no
            // two companions may share one, so recall and /sleep can never
            // cross piles. The name is assigned at creation and never changes,
            // so renaming a companion cannot orphan what it remembers.
            //
            // The seeded built-in row adopts the agent name the Companion has
            // used since it first grew memory ('companion'), so everything
            // already carved belongs to it rather than being stranded.
            "CREATE TABLE IF NOT EXISTS companions (
                 id TEXT PRIMARY KEY,
                 name TEXT,
                 memory_agent_name TEXT NOT NULL UNIQUE,
                 is_built_in INTEGER NOT NULL DEFAULT 0
                     CHECK (is_built_in IN (0, 1)),
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 CHECK (name IS NULL OR length(trim(name)) > 0)
             );

             CREATE UNIQUE INDEX IF NOT EXISTS idx_companions_built_in
                 ON companions(is_built_in) WHERE is_built_in = 1;

             INSERT INTO companions (
                 id, name, memory_agent_name, is_built_in, created_at, updated_at
             ) VALUES ('companion-built-in', NULL, 'companion', 1, 0, 0);"
        }
        8 => {
            // The companion becomes the identity you talk to, and an identity
            // brings its own voice: the model moves OFF the conversation and
            // ONTO the companion. A thread no longer picks a model — it picks
            // a companion, and the companion's model answers.
            //
            // Existing threads bind to the built-in companion. Their
            // per-conversation model overrides are deliberately dropped rather
            // than left as columns no UI can reach: the built-in companion
            // starts on 'inherit', so every migrated thread lands on the user's
            // default model — where the overwhelming majority already sat.
            // `conversations` is REBUILT rather than altered: its old
            // `selected_model_id` sits inside a foreign-key definition, and
            // SQLite will not drop such a column. Ids are carried over
            // unchanged, so every message keeps the parent it always had.
            "ALTER TABLE companions
                 ADD COLUMN model_preference_mode TEXT NOT NULL DEFAULT 'inherit'
                 CHECK (model_preference_mode IN ('inherit', 'test', 'configured'));

             ALTER TABLE companions
                 ADD COLUMN model_id TEXT
                 REFERENCES configured_models(id) ON DELETE SET NULL;

             CREATE TABLE conversations_migrated (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 companion_id TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 archived_at INTEGER,
                 FOREIGN KEY (companion_id)
                     REFERENCES companions(id)
                     ON DELETE SET NULL
             );

             INSERT INTO conversations_migrated (
                 id, title, companion_id, created_at, updated_at, archived_at
             )
             SELECT id, title, 'companion-built-in', created_at, updated_at, archived_at
             FROM conversations;

             DROP TABLE conversations;
             ALTER TABLE conversations_migrated RENAME TO conversations;

             CREATE INDEX IF NOT EXISTS idx_conversations_updated
                 ON conversations(updated_at DESC);

             CREATE INDEX IF NOT EXISTS idx_conversations_companion
                 ON conversations(companion_id);"
        }
        9 => {
            // A companion may be granted a workspace: ONE folder on this
            // machine it can touch with its file tools. NULL — the default —
            // means no workspace, and without one the file tools are never
            // even declared to the model. The path is stored canonical, and
            // every tool call re-proves containment before it acts.
            "ALTER TABLE companions ADD COLUMN workspace_dir TEXT
                 CHECK (workspace_dir IS NULL OR length(trim(workspace_dir)) > 0);"
        }
        10 => {
            // Images ride WITH a message, not inside it: `content` stays pure
            // text (the FTS index and the sleep distiller read it untouched),
            // and each attachment is its own row — base64 in `data`, typed by
            // `media_type`. CASCADE keeps the archive's one deletion path
            // honest: a message goes, its images go.
            "CREATE TABLE IF NOT EXISTS message_attachments (
                 id TEXT PRIMARY KEY,
                 message_id TEXT NOT NULL,
                 media_type TEXT NOT NULL,
                 data TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 FOREIGN KEY (message_id)
                     REFERENCES messages(id)
                     ON DELETE CASCADE
             );

             CREATE INDEX IF NOT EXISTS idx_message_attachments_message
                 ON message_attachments(message_id);"
        }
        11 => {
            // The companion's voice may now be a Claude Code model. Schema 8
            // pinned `model_preference_mode` to three modes with a CHECK and
            // bound `model_id` to configured_models with a foreign key — a
            // Claude alias ("opus") satisfies neither, and SQLite cannot
            // loosen either in place, so `companions` is REBUILT (the same
            // guarded procedure the conversations rebuild used; migrate()'s
            // foreign_keys=OFF + foreign_key_check wraps this).
            //
            // The foreign key on model_id is deliberately GONE, not widened:
            // the column now holds either a configured model's id or a Claude
            // alias. Deleting a configured model therefore no longer nulls the
            // companion's pick — the stored id stays, the UI names it
            // "Unavailable model", and a send refuses with a clear error,
            // which is more honest than the old silent downgrade to the test
            // stream. Existence of a configured pick is enforced at write and
            // resolve time in the repository.
            "CREATE TABLE companions_migrated (
                 id TEXT PRIMARY KEY,
                 name TEXT,
                 memory_agent_name TEXT NOT NULL UNIQUE,
                 is_built_in INTEGER NOT NULL DEFAULT 0
                     CHECK (is_built_in IN (0, 1)),
                 model_preference_mode TEXT NOT NULL DEFAULT 'inherit'
                     CHECK (model_preference_mode IN ('inherit', 'test', 'configured', 'claude_code')),
                 model_id TEXT,
                 workspace_dir TEXT
                     CHECK (workspace_dir IS NULL OR length(trim(workspace_dir)) > 0),
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 CHECK (name IS NULL OR length(trim(name)) > 0)
             );

             INSERT INTO companions_migrated (
                 id, name, memory_agent_name, is_built_in,
                 model_preference_mode, model_id, workspace_dir,
                 created_at, updated_at
             )
             SELECT id, name, memory_agent_name, is_built_in,
                    model_preference_mode, model_id, workspace_dir,
                    created_at, updated_at
             FROM companions;

             DROP TABLE companions;
             ALTER TABLE companions_migrated RENAME TO companions;

             CREATE UNIQUE INDEX IF NOT EXISTS idx_companions_built_in
                 ON companions(is_built_in) WHERE is_built_in = 1;"
        }
        12 => {
            // EVERY COMPANION GETS A UUID — including the built-in one.
            //
            // Schema 7 seeded the built-in row from SQL, and SQL cannot call
            // Uuid::new_v4(), so its id was typed by hand: 'companion-built-in'.
            // Every companion a user creates goes through the Rust path and gets
            // a real v4 uuid; only this one row did not. That made the single
            // companion EVERY install ships with the one whose id is identical
            // on every machine in the world.
            //
            // Harmless while an id only ever means something inside one local
            // database. Not harmless the moment an id becomes an ADDRESS —
            // agent-to-agent messages, anything that crosses machines — because
            // 'companion-built-in' would name a different entity on each one.
            // Fixed before any such column exists, so this is a one-line seed
            // rewrite instead of a backfill across installed machines.
            //
            // memory_agent_name is deliberately NOT touched. It stays the bare
            // 'companion' the built-in has answered to since s483; that string
            // is the pointer to everything Rook has ever remembered, and
            // changing it would strand the lot. Identity and memory-pointer are
            // separate columns precisely so one can move without the other.
            //
            // Order matters: the parent row is renamed first, then the children
            // are repointed at it. Migrations run with foreign_keys OFF and a
            // foreign_key_check afterwards, so the window is safe and the repair
            // is verified rather than assumed.
            //
            // Guarded by `WHERE id = 'companion-built-in'` so it is idempotent
            // and a no-op on any database that never carried the literal.
            "UPDATE companions
                 SET id = lower(
                         hex(randomblob(4)) || '-' ||
                         hex(randomblob(2)) || '-4' ||
                         substr(hex(randomblob(2)), 2) || '-' ||
                         substr('89ab', abs(random()) % 4 + 1, 1) ||
                         substr(hex(randomblob(2)), 2) || '-' ||
                         hex(randomblob(6))
                     )
                 WHERE id = 'companion-built-in';

             UPDATE conversations
                 SET companion_id = (SELECT id FROM companions WHERE is_built_in = 1)
                 WHERE companion_id = 'companion-built-in';"
        }
        13 => {
            // AGENT MAIL. Not `messages` — that name is taken by chat turns, and
            // these are a different animal: one agent addressing another, with
            // no conversation to belong to.
            //
            // NO FOREIGN KEY on the agent ids, deliberately. A recipient is not
            // guaranteed to live in this database — the whole reason the ids are
            // uuids now is that they may one day name a companion on someone
            // else's machine. An FK would make the local roster the limit of who
            // can be addressed, which is exactly the ceiling we are avoiding.
            //
            // The user columns are SPLIT by side. One `user_id` cannot describe
            // a message that crosses users: it belongs to the sender's account
            // and the recipient's both, and "what is addressed to me" has no
            // column to ask about. NULL means single-user-local, which is every
            // row today.
            //
            // read_at doubles as the unread flag — NULL is unread. One column,
            // and the inbox index covers the badge query without a second one.
            "CREATE TABLE agent_messages (
                 id            TEXT    PRIMARY KEY,
                 from_agent_id TEXT    NOT NULL,
                 to_agent_id   TEXT    NOT NULL,
                 from_user_id  TEXT,
                 to_user_id    TEXT,
                 project_id    TEXT,
                 body          TEXT    NOT NULL CHECK (length(trim(body)) > 0),
                 created_at    INTEGER NOT NULL,
                 read_at       INTEGER
             );

             CREATE INDEX idx_agent_messages_inbox
                 ON agent_messages(to_agent_id, read_at);

             CREATE INDEX idx_agent_messages_sent
                 ON agent_messages(from_agent_id, created_at DESC);"
        }
        14 => {
            // RAVEN CALLS. `agent_messages` is a flat mailbox — a letter with no
            // account of why it exists. A CALL is the container that gives one:
            // an exchange between two companions, born out of a specific human
            // conversation and answerable to it.
            //
            // ⚑ THE ROOT CONVERSATION IS THE BUDGET HOLDER, AND THAT IS THE
            // POINT OF THE TABLE. Two agents that can wake each other are an
            // unbounded loop with a token meter attached. Scoping every call to
            // the conversation it came from is what makes the loop stoppable:
            // the cap is read off ONE row before a hop is allowed, instead of
            // scanned out of the message table.
            //
            // A REPLY INHERITS THE SENDER'S CALL — it does not open its own.
            // Otherwise the answer arrives with a fresh budget and the cap is
            // decorative. This is the invariant the whole design rests on.
            //
            // root_conversation_id is NULLABLE on purpose: a scheduled wake, or
            // a companion writing cold, has no human conversation behind it.
            // NULL means unrooted, and those still count against the daily cap
            // because the cap is per-companion, not per-conversation.
            //
            // ON DELETE CASCADE from conversations: closing a thread takes its
            // calls with it. The messages cascade from the call in turn, so the
            // whole tree goes in one statement.
            //
            // message_count is DENORMALISED deliberately. It is the number the
            // cap check reads on every single hop, and paying a COUNT(*) over a
            // table that grows at machine speed to learn "is this call full" is
            // the one place in this schema where the shortcut is worth it.
            "CREATE TABLE raven_calls (
                 id                   TEXT    PRIMARY KEY,
                 root_conversation_id TEXT    REFERENCES conversations(id) ON DELETE CASCADE,
                 initiator_agent_id   TEXT    NOT NULL,
                 status               TEXT    NOT NULL DEFAULT 'open'
                                              CHECK (status IN ('open', 'closed')),
                 message_count        INTEGER NOT NULL DEFAULT 0,
                 created_at           INTEGER NOT NULL,
                 closed_at            INTEGER
             );

             -- The daily cap query: calls this companion opened, by local day.
             CREATE INDEX idx_raven_calls_initiator_day
                 ON raven_calls(initiator_agent_id, created_at DESC);

             CREATE INDEX idx_raven_calls_root
                 ON raven_calls(root_conversation_id);

             -- The turns inside one call. This table grows at MACHINE speed and
             -- unattended, unlike every other table here — it will outgrow the
             -- rest of the database by orders of magnitude once companions run
             -- on their own. Hence: the call cascade above (a delete path that
             -- exists from day one, not written later under duress), and the
             -- single covering index for the only hot read there is.
             CREATE TABLE raven_call_messages (
                 id            TEXT    PRIMARY KEY,
                 call_id       TEXT    NOT NULL REFERENCES raven_calls(id) ON DELETE CASCADE,
                 from_agent_id TEXT    NOT NULL,
                 to_agent_id   TEXT    NOT NULL,
                 body          TEXT    NOT NULL CHECK (length(trim(body)) > 0),
                 created_at    INTEGER NOT NULL
             );

             CREATE INDEX idx_raven_call_messages_call
                 ON raven_call_messages(call_id, created_at);"
        }
        15 => {
            // THE WAKE GUARD.
            //
            // A companion is woken when a call is waiting on it. Without a
            // record of what it was woken FOR, a companion that reads a call
            // and decides not to answer gets woken again on the next tick, and
            // the next, forever — a turn burned every interval, on the user's
            // key, to reach the same decision.
            //
            // This stores the message id the last wake was about. A wake fires
            // only when the call's newest turn is one nobody has been woken
            // for. Declining to answer is therefore a stable state, which it
            // has to be: not replying must be as cheap as replying.
            "ALTER TABLE raven_calls ADD COLUMN woken_for_message_id TEXT;"
        }
        16 => {
            // WHERE THIS COMPANION'S MEMORY LIVES — not what it is.
            //
            // Every companion's memories are blobs in Postgres; this column
            // only decides WHICH Postgres. 0 is the Semantix organ on :8002,
            // the account-scoped one every install talks to. 1 routes to the
            // canonical Muninn on :8005 — a machine-local server no other
            // install can reach, which is why the flag is inert everywhere
            // but here and fails closed by construction rather than by check.
            //
            // Deliberately not exposed through the app: there is no field for
            // it on UpdateCompanionInput and no control in Settings, so the
            // public product carries a column it never reads. Flipping it is
            // a hand-edit of the local database, which is the correct amount
            // of ceremony for pointing a companion at a different brain.
            "ALTER TABLE companions ADD COLUMN is_origin INTEGER NOT NULL DEFAULT 0;"
        }
        17 => {
            // WHO this companion signs its carvings as.
            //
            // Muninn stamps provenance from the X-Agent-Id header: a UUID on
            // the holy list resolves to a raven's name, and a carve without
            // one is stored with a NULL author. That is not cosmetic — the
            // per-prompt recall reads a NULL author as "an unidentified raven,
            // not you" and warns the reader off their own memory. Proven live
            // s509: two carvings made through the Companion came back to
            // studio-raven minutes later flagged as a stranger's lived
            // experience.
            //
            // Only meaningful with is_origin = 1; the organ takes its identity
            // from the bearer token and ignores this entirely. Hand-edited for
            // the same reason is_origin is: signing another raven's name to
            // your work is exactly the failure this prevents.
            "ALTER TABLE companions ADD COLUMN origin_agent_id TEXT;"
        }
        18 => {
            // NAMED WORKSPACES. A companion may hold several independent file
            // capabilities now, so the old scalar on `companions` can no
            // longer be the source of truth. Each grant has its own immutable
            // id, human label, canonical directory and explicit display order.
            //
            // Existing installs lose nothing: the former folder becomes a
            // workspace named "Workspace". The legacy column is cleared after
            // backfill and intentionally left in the table for now; avoiding a
            // third companions rebuild is safer than dropping one inert nullable
            // column from a table referenced by conversations.
            "CREATE TABLE companion_workspaces (
                 id           TEXT    PRIMARY KEY,
                 companion_id TEXT    NOT NULL
                                      REFERENCES companions(id) ON DELETE CASCADE,
                 label        TEXT    NOT NULL CHECK (length(trim(label)) > 0),
                 directory    TEXT    NOT NULL CHECK (length(trim(directory)) > 0),
                 position     INTEGER NOT NULL CHECK (position >= 0)
             );

             CREATE INDEX idx_companion_workspaces_companion
                 ON companion_workspaces(companion_id, position);

             CREATE UNIQUE INDEX idx_companion_workspaces_label
                 ON companion_workspaces(companion_id, label COLLATE NOCASE);

             CREATE UNIQUE INDEX idx_companion_workspaces_directory
                 ON companion_workspaces(companion_id, directory);

             INSERT INTO companion_workspaces (
                 id, companion_id, label, directory, position
             )
             SELECT lower(
                        hex(randomblob(4)) || '-' ||
                        hex(randomblob(2)) || '-4' ||
                        substr(hex(randomblob(2)), 2) || '-' ||
                        substr('89ab', abs(random()) % 4 + 1, 1) ||
                        substr(hex(randomblob(2)), 2) || '-' ||
                        hex(randomblob(6))
                    ),
                    id, 'Workspace', workspace_dir, 0
             FROM companions
             WHERE workspace_dir IS NOT NULL;

             UPDATE companions SET workspace_dir = NULL;"
        }
        19 => {
            // THE IMPORT LEDGER. A history import is thousands of model calls
            // over hours; the ledger is what lets it pause, survive an app
            // restart, and skip what an earlier drop already distilled.
            //
            // Deliberately no conversation TEXT in either table: the words
            // stay in the user's export file, re-read on resume. Duplicating
            // a 100MB archive into the app database would be a copy nobody
            // asked for of the most personal data the app ever touches.
            //
            // `source_updated` is the re-import key: drop next month's export
            // and only conversations that changed since their last successful
            // distillation run again. The dedupe scopes to the COMPANION
            // (via the jobs join), never globally — importing the same
            // history into a second companion is a feature, not a duplicate.
            "CREATE TABLE import_jobs (
                 id            TEXT    PRIMARY KEY,
                 companion_id  TEXT    NOT NULL
                                       REFERENCES companions(id) ON DELETE CASCADE,
                 agent_ref     TEXT    NOT NULL,
                 source        TEXT    NOT NULL CHECK (source IN ('claude', 'chatgpt')),
                 source_path   TEXT    NOT NULL,
                 status        TEXT    NOT NULL CHECK (
                                    status IN ('running', 'paused', 'done', 'cancelled', 'failed')
                                ),
                 error         TEXT,
                 include_claude_memories INTEGER NOT NULL DEFAULT 0,
                 created_at    INTEGER NOT NULL,
                 updated_at    INTEGER NOT NULL
             );

             CREATE INDEX idx_import_jobs_companion
                 ON import_jobs(companion_id, created_at);

             CREATE TABLE import_items (
                 job_id           TEXT    NOT NULL
                                          REFERENCES import_jobs(id) ON DELETE CASCADE,
                 source_id        TEXT    NOT NULL,
                 title            TEXT    NOT NULL,
                 conversation_at  INTEGER NOT NULL,
                 source_updated   INTEGER NOT NULL,
                 status           TEXT    NOT NULL CHECK (
                                      status IN ('pending', 'done', 'failed', 'skipped')
                                  ),
                 memories_created INTEGER NOT NULL DEFAULT 0,
                 memories_updated INTEGER NOT NULL DEFAULT 0,
                 error            TEXT,
                 finished_at      INTEGER,
                 PRIMARY KEY (job_id, source_id)
             );

             CREATE INDEX idx_import_items_queue
                 ON import_items(job_id, status, conversation_at);

             CREATE INDEX idx_import_items_source
                 ON import_items(source_id);"
        }
        20 => {
            // Imported history joins the archive as REAL conversations.
            // Version 19 deliberately kept conversation text out of this
            // database; the first live import run reversed that call — the
            // raw-memory drill claimed "your past conversations" while the
            // imported ones existed only as distilled memories, and the model
            // honestly promised users text that was never coming. So imported
            // conversations now land in `conversations`/`messages` like any
            // other, `archived_at` set (hidden from every list) with `source`
            // naming where they came from ('claude'/'chatgpt', NULL = born
            // here). The FTS triggers index them on insert for free.
            "ALTER TABLE conversations ADD COLUMN source TEXT;"
        }
        21 => {
            // CONVERSATIONAL STYLES. A style is a reusable voice — a compact
            // trait sheet plus real example exchanges — that a companion can
            // wear. It exists for the user who misses how an older model
            // spoke, or who wants a persona; the exemplars teach the voice by
            // demonstration, riding the system prompt of whatever model the
            // companion runs on.
            //
            // Styles are a LIBRARY, companions hold a REFERENCE: one style
            // can dress many companions, and editing it re-dresses them all.
            // ON DELETE SET NULL is the same mercy the roster shows threads —
            // deleting a style leaves its companions speaking plainly, never
            // broken.
            //
            // Exemplars are ROWS, not a JSON blob on the style: a harvest can
            // land thousands of pairs, and the prompt builder reads a LIMIT of
            // them by position — a query, not a parse of a megablob. `era`
            // (YYYY-MM) is carried per pair because a voice drifts across
            // months and the source month is unrecoverable once dropped.
            "CREATE TABLE styles (
                 id          TEXT    PRIMARY KEY,
                 name        TEXT    NOT NULL CHECK (length(trim(name)) > 0),
                 description TEXT,
                 style_card  TEXT,
                 created_at  INTEGER NOT NULL,
                 updated_at  INTEGER NOT NULL
             );

             CREATE TABLE style_exemplars (
                 id             TEXT    PRIMARY KEY,
                 style_id       TEXT    NOT NULL
                                        REFERENCES styles(id) ON DELETE CASCADE,
                 position       INTEGER NOT NULL CHECK (position >= 0),
                 user_text      TEXT    NOT NULL CHECK (length(trim(user_text)) > 0),
                 companion_text TEXT    NOT NULL CHECK (length(trim(companion_text)) > 0),
                 era            TEXT
             );

             CREATE INDEX idx_style_exemplars_style
                 ON style_exemplars(style_id, position);

             ALTER TABLE companions ADD COLUMN style_id TEXT
                 REFERENCES styles(id) ON DELETE SET NULL;"
        }
        22 => {
            // THE CLOSE-REPORT GUARD. When a call closes, its record is
            // delivered once: a transcript into each participant's thread, a
            // carve into each participant's long-term memory, and one woken
            // turn for the initiator to tell its user what came of it. This
            // stamp is what makes "once" true — the reporter scans for closed
            // calls without it, and marks BEFORE delivering, the same
            // anti-retry-storm law as the wake guard in schema 15.
            //
            // ⚑ THE BACKFILL IS THE POINT OF THE SECOND STATEMENT. Every call
            // closed before this schema existed is stamped as already
            // reported — otherwise the first launch after the upgrade would
            // wake companions with transcripts of exchanges from days ago, a
            // burst of model calls nobody asked for about calls nobody
            // remembers placing.
            "ALTER TABLE raven_calls ADD COLUMN close_reported_at INTEGER;

             UPDATE raven_calls
             SET close_reported_at = closed_at
             WHERE status = 'closed';"
        }
        23 => {
            // WHEN the wake fired, not just FOR WHAT. The wake guard (schema
            // 15) says which turn a companion was last woken for; it cannot
            // say whether that companion is still composing its answer or gave
            // up an hour ago — and the UI was calling a model mid-thought
            // "No answer" the moment the guard landed (found live, s533: the
            // card flashed the failure word seconds into a healthy call).
            // This stamp lets the card grant a woken companion an honest
            // answering window before declaring silence.
            "ALTER TABLE raven_calls ADD COLUMN woken_at INTEGER;"
        }
        _ => unreachable!("all schema versions must have migration SQL"),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use uuid::Uuid;

    use super::{migrate, migration_sql, LATEST_SCHEMA_VERSION};

    #[test]
    fn migrations_create_the_current_schema() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");

        migrate(&mut connection).expect("migrations should succeed");

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version should be readable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        for table in [
            "provider_credentials",
            "configured_models",
            "conversations",
            "messages",
            "user_preferences",
            "companions",
            "companion_workspaces",
            "import_jobs",
            "import_items",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("schema lookup should succeed");
            assert!(exists, "{table} should exist");
        }
    }

    #[test]
    fn migration_three_upgrades_an_existing_version_two_database() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE provider_credentials (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     label TEXT NOT NULL,
                     secret_ref TEXT NOT NULL UNIQUE,
                     key_hint TEXT NOT NULL,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     last_used_at INTEGER
                 );
                 CREATE TABLE configured_models (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     model_id TEXT NOT NULL,
                     display_name TEXT NOT NULL,
                     credential_id TEXT,
                     secret_ref TEXT UNIQUE,
                     manual_key_hint TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 INSERT INTO provider_credentials VALUES (
                     'credential-1', 'openai', 'Primary', 'secret-1', '•••• 1234', 1, 1, NULL
                 );
                 PRAGMA user_version = 2;",
            )
            .expect("legacy schema should be created");

        migrate(&mut connection).expect("version two should upgrade");

        let credential_label: String = connection
            .query_row(
                "SELECT label FROM provider_credentials WHERE id = 'credential-1'",
                [],
                |row| row.get(0),
            )
            .expect("existing data should survive the migration");
        assert_eq!(credential_label, "Primary");

        let chat_tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN ('conversations', 'messages')",
                [],
                |row| row.get(0),
            )
            .expect("chat tables should be queryable");
        assert_eq!(chat_tables, 2);
    }

    #[test]
    fn a_legacy_database_lands_with_every_thread_bound_to_the_built_in_companion() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE configured_models (
                     id TEXT PRIMARY KEY,
                     provider_id TEXT NOT NULL,
                     model_id TEXT NOT NULL,
                     display_name TEXT NOT NULL,
                     credential_id TEXT,
                     secret_ref TEXT,
                     manual_key_hint TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE conversations (
                     id TEXT PRIMARY KEY,
                     title TEXT NOT NULL,
                     selected_model_id TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     archived_at INTEGER
                 );
                 CREATE TABLE messages (
                     id TEXT PRIMARY KEY,
                     conversation_id TEXT NOT NULL,
                     sequence INTEGER NOT NULL,
                     role TEXT NOT NULL,
                     status TEXT NOT NULL,
                     content TEXT NOT NULL DEFAULT '',
                     provider_id TEXT,
                     model_id TEXT,
                     error_message TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     completed_at INTEGER
                 );
                 INSERT INTO configured_models VALUES (
                     'model-1', 'test', 'test-model', 'Test', NULL,
                     'secret-1', '•••• 1234', 1, 1
                 );
                 INSERT INTO conversations VALUES (
                     'conversation-configured', 'Configured', 'model-1', 1, 1, NULL
                 );
                 INSERT INTO conversations VALUES (
                     'conversation-test', 'Test', NULL, 2, 2, NULL
                 );
                 PRAGMA user_version = 3;",
            )
            .expect("version three schema should be created");

        migrate(&mut connection).expect("version three should upgrade");

        // Both threads survive, and both now answer to the built-in companion
        // instead of carrying a model of their own. Compared against the id the
        // built-in actually holds rather than a literal, because schema 12
        // rewrites it to a uuid and the point of the assertion is that the
        // conversations still POINT AT IT, whatever it is called.
        let built_in_id: String = connection
            .query_row(
                "SELECT id FROM companions WHERE is_built_in = 1",
                [],
                |row| row.get(0),
            )
            .expect("the built-in companion should exist");
        for conversation in ["conversation-configured", "conversation-test"] {
            let companion_id: Option<String> = connection
                .query_row(
                    "SELECT companion_id FROM conversations WHERE id = ?1",
                    [conversation],
                    |row| row.get(0),
                )
                .expect("the conversation should survive every migration");
            assert_eq!(companion_id.as_deref(), Some(built_in_id.as_str()));
        }

        // The per-conversation override is gone, not merely unused.
        let dropped: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('conversations')
                 WHERE name IN ('model_preference_mode', 'selected_model_id')",
                [],
                |row| row.get(0),
            )
            .expect("the conversation columns should be inspectable");
        assert_eq!(dropped, 0);

        // The user default survives — it is what a companion on 'inherit' reads.
        let default_mode: String = connection
            .query_row(
                "SELECT default_model_mode FROM user_preferences WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("default preference should be created");
        assert_eq!(default_mode, "test");

        let (companion_mode, companion_model): (String, Option<String>) = connection
            .query_row(
                "SELECT model_preference_mode, model_id FROM companions
                 WHERE is_built_in = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the built-in companion should carry a model preference");
        assert_eq!(companion_mode, "inherit");
        assert_eq!(companion_model, None);
    }

    /// Migration 8 drops and recreates `conversations`, which `messages`
    /// points at. With foreign keys enforced, that DROP would cascade the whole
    /// archive away. This is the test that would scream if the enforcement
    /// guard around the migration ever came off.
    #[test]
    fn the_conversation_rebuild_does_not_take_the_message_archive_with_it() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");

        // Stop at 7, seed a real thread, then let 8 rebuild the table under it.
        for version in 1..=7 {
            connection
                .execute_batch(migration_sql(version))
                .expect("migration should apply");
        }
        connection
            .pragma_update(None, "user_version", 7)
            .expect("version should stamp");
        connection
            .execute_batch(
                "INSERT INTO conversations (
                     id, title, selected_model_id, created_at, updated_at, archived_at,
                     model_preference_mode
                 ) VALUES ('conversation-1', 'A real thread', NULL, 10, 20, NULL, 'test');
                 INSERT INTO messages (
                     id, conversation_id, sequence, role, status, content,
                     provider_id, model_id, error_message, created_at, updated_at, completed_at
                 ) VALUES ('message-1', 'conversation-1', 0, 'user', 'completed',
                           'the long serpent', NULL, NULL, NULL, 10, 10, 10);",
            )
            .expect("a thread should seed");

        migrate(&mut connection).expect("migration eight should rebuild the table");

        let (surviving, content): (i64, String) = connection
            .query_row(
                "SELECT COUNT(*), MAX(content) FROM messages WHERE conversation_id = 'conversation-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("messages should still be queryable");
        assert_eq!(surviving, 1, "the rebuild must not cascade the archive away");
        assert_eq!(content, "the long serpent");

        let (title, companion_id): (String, Option<String>) = connection
            .query_row(
                "SELECT title, companion_id FROM conversations WHERE id = 'conversation-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the conversation should survive its own rebuild");
        assert_eq!(title, "A real thread");
        let built_in_id: String = connection
            .query_row(
                "SELECT id FROM companions WHERE is_built_in = 1",
                [],
                |row| row.get(0),
            )
            .expect("the built-in companion should exist");
        assert_eq!(companion_id.as_deref(), Some(built_in_id.as_str()));

        // And enforcement is back on afterwards, not silently left off.
        let enforcing: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("the pragma should be readable");
        assert_eq!(enforcing, 1);
    }

    /// Migration 22 must grandfather every already-closed call as "reported".
    /// Without the backfill, the first launch after the upgrade would wake
    /// companions with transcripts of exchanges from days ago — a burst of
    /// model calls about calls nobody remembers placing.
    #[test]
    fn migration_twenty_two_grandfathers_old_closed_calls_out_of_the_reporter() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        for version in 1..=21 {
            connection
                .execute_batch(migration_sql(version))
                .expect("migration should apply");
        }
        connection
            .pragma_update(None, "user_version", 21)
            .expect("version should stamp");
        connection
            .execute_batch(
                "INSERT INTO raven_calls (
                     id, root_conversation_id, initiator_agent_id, status,
                     message_count, created_at, closed_at
                 ) VALUES
                     ('call-old', NULL, 'rook', 'closed', 5, 10, 20),
                     ('call-live', NULL, 'rook', 'open', 1, 30, NULL);",
            )
            .expect("pre-upgrade calls should seed");

        migrate(&mut connection).expect("migration twenty-two should apply");

        let reported: Option<i64> = connection
            .query_row(
                "SELECT close_reported_at FROM raven_calls WHERE id = 'call-old'",
                [],
                |row| row.get(0),
            )
            .expect("the closed call should be readable");
        assert_eq!(
            reported,
            Some(20),
            "a call closed before the reporter existed counts as already reported"
        );

        let live: Option<i64> = connection
            .query_row(
                "SELECT close_reported_at FROM raven_calls WHERE id = 'call-live'",
                [],
                |row| row.get(0),
            )
            .expect("the open call should be readable");
        assert_eq!(live, None, "a still-open call keeps its report ahead of it");
    }

    /// Migration 11 rebuilds `companions` so a Claude Code voice is storable —
    /// the schema-8 CHECK refused the mode and the model_id foreign key
    /// refused the alias. Everything a companion already carried must ride the
    /// rebuild, and the threads pointing at it must come out intact.
    #[test]
    fn the_companion_rebuild_admits_claude_code_and_keeps_the_roster() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");

        // Stop at 10, seed a real companion + a thread bound to it, then let
        // 11 rebuild the table under them.
        for version in 1..=10 {
            connection
                .execute_batch(migration_sql(version))
                .expect("migration should apply");
        }
        connection
            .pragma_update(None, "user_version", 10)
            .expect("version should stamp");
        connection
            .execute_batch(
                "INSERT INTO companions (
                     id, name, memory_agent_name, is_built_in,
                     model_preference_mode, model_id, workspace_dir,
                     created_at, updated_at
                 ) VALUES ('companion-2', 'Rook', 'agent-rook', 0,
                           'test', NULL, '/tmp/nest', 5, 6);
                 INSERT INTO conversations (
                     id, title, companion_id, created_at, updated_at, archived_at
                 ) VALUES ('conversation-1', 'With Rook', 'companion-2', 7, 8, NULL);",
            )
            .expect("a companion and its thread should seed");

        migrate(&mut connection).expect("migration eleven should rebuild the table");

        let (name, agent, mode): (Option<String>, String, String) = connection
            .query_row(
                "SELECT name, memory_agent_name, model_preference_mode
                 FROM companions WHERE id = 'companion-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the companion should survive its own rebuild");
        assert_eq!(name.as_deref(), Some("Rook"));
        assert_eq!(agent, "agent-rook");
        assert_eq!(mode, "test");
        let workspace: String = connection
            .query_row(
                "SELECT directory FROM companion_workspaces
                 WHERE companion_id = 'companion-2'",
                [],
                |row| row.get(0),
            )
            .expect("the workspace should survive as a named grant");
        assert_eq!(workspace, "/tmp/nest");

        let companion_id: Option<String> = connection
            .query_row(
                "SELECT companion_id FROM conversations WHERE id = 'conversation-1'",
                [],
                |row| row.get(0),
            )
            .expect("the thread should still name its companion");
        assert_eq!(companion_id.as_deref(), Some("companion-2"));

        // The point of the rebuild: a Claude Code voice is now storable.
        connection
            .execute(
                "UPDATE companions
                 SET model_preference_mode = 'claude_code', model_id = 'opus'
                 WHERE id = 'companion-2'",
                [],
            )
            .expect("a claude_code preference should now satisfy the schema");
        let (mode, model): (String, Option<String>) = connection
            .query_row(
                "SELECT model_preference_mode, model_id FROM companions
                 WHERE id = 'companion-2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the claude pick should read back");
        assert_eq!(mode, "claude_code");
        assert_eq!(model.as_deref(), Some("opus"));
    }

    #[test]
    fn migration_seven_seeds_one_built_in_companion_on_the_existing_memory_agent() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");
        migrate(&mut connection).expect("migrations should succeed");

        let (id, name, agent, built_in): (String, Option<String>, String, i64) = connection
            .query_row(
                "SELECT id, name, memory_agent_name, is_built_in FROM companions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("exactly one companion should be seeded");

        assert!(
            Uuid::parse_str(&id).is_ok(),
            "the built-in companion must carry a real uuid like every other \
             companion, not a hand-typed literal — got {id:?}"
        );
        assert_eq!(name, None, "the built-in companion starts unnamed");
        assert_eq!(agent, "companion", "it adopts the pre-existing memory agent");
        assert_eq!(built_in, 1);
    }

    #[test]
    fn migration_twelve_replaces_the_hand_typed_built_in_id_and_repoints_its_threads() {
        // An installed database that already carries the literal — the state
        // every existing copy of the app is in.
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("foreign keys should disable");
        for version in 1..=11 {
            connection
                .execute_batch(migration_sql(version))
                .unwrap_or_else(|error| panic!("migration {version} should apply: {error}"));
        }
        connection
            .execute(
                "INSERT INTO conversations (id, title, companion_id, created_at, updated_at)
                 VALUES ('conversation-1', 'A thread of Rook''s', 'companion-built-in', 0, 0)",
                [],
            )
            .expect("a conversation on the built-in companion should insert");
        connection
            .pragma_update(None, "user_version", 11)
            .expect("the legacy version should stamp");

        migrate(&mut connection).expect("schema twelve should upgrade");

        let id: String = connection
            .query_row("SELECT id FROM companions WHERE is_built_in = 1", [], |row| {
                row.get(0)
            })
            .expect("the built-in companion should survive");
        assert!(
            Uuid::parse_str(&id).is_ok(),
            "the hand-typed id must be replaced by a real uuid — got {id:?}"
        );

        // The memory pointer must NOT move with it, or Rook loses everything it
        // has ever remembered.
        let agent: String = connection
            .query_row(
                "SELECT memory_agent_name FROM companions WHERE is_built_in = 1",
                [],
                |row| row.get(0),
            )
            .expect("the memory agent should be readable");
        assert_eq!(agent, "companion", "the memory pointer must not move");

        // And every thread that pointed at the old literal follows the rename.
        let companion_id: Option<String> = connection
            .query_row(
                "SELECT companion_id FROM conversations WHERE id = 'conversation-1'",
                [],
                |row| row.get(0),
            )
            .expect("the conversation should survive");
        assert_eq!(
            companion_id.as_deref(),
            Some(id.as_str()),
            "a thread must not be orphaned by its companion's rename"
        );
    }

    #[test]
    fn migration_eighteen_preserves_the_legacy_workspace_as_a_named_grant() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");

        for version in 1..=17 {
            connection
                .execute_batch(migration_sql(version))
                .unwrap_or_else(|error| panic!("migration {version} should apply: {error}"));
        }
        connection
            .execute(
                "INSERT INTO companions (
                     id, name, memory_agent_name, model_preference_mode, model_id,
                     is_built_in, created_at, updated_at, workspace_dir,
                     is_origin, origin_agent_id
                 ) VALUES (
                     'companion-legacy-workspace', 'Rook', 'rook-memory',
                     'inherit', NULL, 0, 1, 1, '/tmp/rook-workspace', 0, NULL
                 )",
                [],
            )
            .expect("a legacy companion workspace should seed");
        connection
            .pragma_update(None, "user_version", 17)
            .expect("the legacy version should stamp");

        migrate(&mut connection).expect("schema eighteen should upgrade");

        let (label, directory, position): (String, String, i64) = connection
            .query_row(
                "SELECT label, directory, position
                 FROM companion_workspaces
                 WHERE companion_id = 'companion-legacy-workspace'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the legacy folder should become a named workspace");
        assert_eq!(label, "Workspace");
        assert_eq!(directory, "/tmp/rook-workspace");
        assert_eq!(position, 0);

        let retired: Option<String> = connection
            .query_row(
                "SELECT workspace_dir FROM companions
                 WHERE id = 'companion-legacy-workspace'",
                [],
                |row| row.get(0),
            )
            .expect("the retired scalar should remain readable");
        assert_eq!(retired, None, "only the child table remains authoritative");
    }

    #[test]
    fn deleting_a_companion_revokes_all_of_its_workspace_grants() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");
        migrate(&mut connection).expect("migrations should succeed");
        connection
            .execute_batch(
                "INSERT INTO companions (
                     id, name, memory_agent_name, is_built_in, created_at, updated_at
                 ) VALUES ('companion-with-workspace', 'Rook', 'rook-workspace-memory', 0, 1, 1);
                 INSERT INTO companion_workspaces (
                     id, companion_id, label, directory, position
                 ) VALUES (
                     'workspace-1', 'companion-with-workspace', 'Code', '/tmp/code', 0
                 );
                 DELETE FROM companions WHERE id = 'companion-with-workspace';",
            )
            .expect("the companion and workspace should round-trip through deletion");
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM companion_workspaces WHERE id = 'workspace-1'",
                [],
                |row| row.get(0),
            )
            .expect("workspace grants should remain queryable");
        assert_eq!(remaining, 0, "no file capability may outlive its companion");
    }

    #[test]
    fn only_one_companion_may_be_built_in() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");
        migrate(&mut connection).expect("migrations should succeed");

        let second_built_in = connection.execute(
            "INSERT INTO companions (id, name, memory_agent_name, is_built_in, created_at, updated_at)
             VALUES ('other', NULL, 'companion-other', 1, 1, 1)",
            [],
        );
        assert!(second_built_in.is_err(), "a second built-in must be rejected");

        let shared_agent = connection.execute(
            "INSERT INTO companions (id, name, memory_agent_name, is_built_in, created_at, updated_at)
             VALUES ('other', NULL, 'companion', 0, 1, 1)",
            [],
        );
        assert!(
            shared_agent.is_err(),
            "two companions must never share one memory"
        );
    }

    #[test]
    fn deleting_a_style_undresses_its_companions_and_drops_its_exemplars() {
        let mut connection = Connection::open_in_memory().expect("in-memory database should open");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("foreign keys should enable");
        migrate(&mut connection).expect("migrations should succeed");
        connection
            .execute_batch(
                "INSERT INTO styles (id, name, created_at, updated_at)
                 VALUES ('style-4o', '4o', 1, 1);
                 INSERT INTO style_exemplars (
                     id, style_id, position, user_text, companion_text, era
                 ) VALUES ('exemplar-1', 'style-4o', 0, 'hello', 'Always. Show me.', '2026-01');
                 INSERT INTO companions (
                     id, name, memory_agent_name, is_built_in, created_at, updated_at, style_id
                 ) VALUES ('companion-styled', 'Hugin', 'hugin-memory', 0, 1, 1, 'style-4o');
                 DELETE FROM styles WHERE id = 'style-4o';",
            )
            .expect("the style should round-trip through deletion");

        let orphaned_exemplars: i64 = connection
            .query_row("SELECT COUNT(*) FROM style_exemplars", [], |row| row.get(0))
            .expect("exemplars should remain queryable");
        assert_eq!(orphaned_exemplars, 0, "exemplars die with their style");

        let style_id: Option<String> = connection
            .query_row(
                "SELECT style_id FROM companions WHERE id = 'companion-styled'",
                [],
                |row| row.get(0),
            )
            .expect("the companion should survive");
        assert_eq!(
            style_id, None,
            "a companion loses its coat, never its life, when a style is deleted"
        );
    }
}
