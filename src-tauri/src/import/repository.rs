// The import ledger — what makes a thousands-of-calls import pausable,
// restart-safe, and honest about what already happened.
//
// A job is one drop of one export into one companion. Its items are the
// conversations, written up-front as `pending` and consumed oldest-first; the
// conversation TEXT is never here (re-read from the export on resume — see the
// schema 19 note). Cross-job dedupe happens at item insert: a conversation an
// earlier job already distilled into the SAME companion, unchanged since
// (`source_updated`), is born `skipped` instead of `pending` — so dropping
// next month's export costs only what actually changed.

use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::ImportSource;
use crate::{app_error::AppError, database};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum JobStatus {
    Running,
    Paused,
    Done,
    Cancelled,
    Failed,
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn from_storage(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "done" => Self::Done,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }
}

impl ImportSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ChatGpt => "chatgpt",
        }
    }

    fn from_storage(value: &str) -> Self {
        match value {
            "claude" => Self::Claude,
            _ => Self::ChatGpt,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportJob {
    pub(crate) id: String,
    pub(crate) companion_id: String,
    pub(crate) agent_ref: String,
    pub(crate) source: ImportSource,
    pub(crate) source_path: String,
    pub(crate) status: JobStatus,
    pub(crate) error: Option<String>,
    pub(crate) include_claude_memories: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// The job with its progress counts — what the wizard's progress card renders.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportJobSnapshot {
    #[serde(flatten)]
    pub(crate) job: ImportJob,
    pub(crate) pending: i64,
    pub(crate) done: i64,
    pub(crate) failed: i64,
    pub(crate) skipped: i64,
    pub(crate) memories_created: i64,
    pub(crate) memories_updated: i64,
}

pub(crate) struct NewImportItem {
    pub(crate) source_id: String,
    pub(crate) title: String,
    pub(crate) conversation_at: i64,
    pub(crate) source_updated: i64,
}

/// The next conversation the worker should feed the distiller.
pub(crate) struct PendingItem {
    pub(crate) source_id: String,
    pub(crate) title: String,
    pub(crate) conversation_at: i64,
}

pub(crate) struct ImportRepository {
    connection: Mutex<Connection>,
}

impl ImportRepository {
    pub(crate) fn open(path: &Path) -> Result<Self, AppError> {
        Ok(Self {
            connection: Mutex::new(database::open_connection(path)?),
        })
    }

    /// Create a job with its whole item list in one transaction. Items whose
    /// conversation an earlier job already distilled into this companion —
    /// same `source_id`, not newer than what was done — are born `skipped`.
    pub(crate) fn create_job(
        &self,
        job: &ImportJob,
        items: &[NewImportItem],
    ) -> Result<(), AppError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction().map_err(AppError::database)?;
        transaction
            .execute(
                "INSERT INTO import_jobs (
                     id, companion_id, agent_ref, source, source_path, status,
                     error, include_claude_memories, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    job.id,
                    job.companion_id,
                    job.agent_ref,
                    job.source.as_str(),
                    job.source_path,
                    job.status.as_str(),
                    job.error,
                    i64::from(job.include_claude_memories),
                    job.created_at,
                    job.updated_at,
                ],
            )
            .map_err(AppError::database)?;
        for item in items {
            transaction
                .execute(
                    "INSERT INTO import_items (
                         job_id, source_id, title, conversation_at, source_updated, status
                     ) VALUES (
                         ?1, ?2, ?3, ?4, ?5,
                         CASE WHEN EXISTS (
                             SELECT 1
                             FROM import_items done_item
                             JOIN import_jobs done_job ON done_item.job_id = done_job.id
                             WHERE done_job.companion_id = ?6
                               AND done_item.source_id = ?2
                               AND done_item.status = 'done'
                               AND done_item.source_updated >= ?5
                         ) THEN 'skipped' ELSE 'pending' END
                     )",
                    params![
                        job.id,
                        item.source_id,
                        item.title,
                        item.conversation_at,
                        item.source_updated,
                        job.companion_id,
                    ],
                )
                .map_err(AppError::database)?;
        }
        transaction.commit().map_err(AppError::database)?;
        Ok(())
    }

    /// Every conversation this job covers, whatever its status — `skipped`
    /// included, because a re-drop still archives what an earlier job already
    /// distilled. This is the archive pass's scope.
    pub(crate) fn item_source_ids(&self, job_id: &str) -> Result<Vec<String>, AppError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare("SELECT source_id FROM import_items WHERE job_id = ?1")
            .map_err(AppError::database)?;
        let ids = statement
            .query_map([job_id], |row| row.get(0))
            .map_err(AppError::database)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(AppError::database)?;
        Ok(ids)
    }

    /// Oldest conversation first — eras accrue forward, matching the order the
    /// export is fed to the distiller.
    pub(crate) fn next_pending(&self, job_id: &str) -> Result<Option<PendingItem>, AppError> {
        self.connection()?
            .query_row(
                "SELECT source_id, title, conversation_at
                 FROM import_items
                 WHERE job_id = ?1 AND status = 'pending'
                 ORDER BY conversation_at ASC, source_id ASC
                 LIMIT 1",
                [job_id],
                |row| {
                    Ok(PendingItem {
                        source_id: row.get(0)?,
                        title: row.get(1)?,
                        conversation_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(AppError::database)
    }

    pub(crate) fn mark_item_done(
        &self,
        job_id: &str,
        source_id: &str,
        memories_created: i64,
        memories_updated: i64,
        finished_at: i64,
    ) -> Result<(), AppError> {
        self.connection()?
            .execute(
                "UPDATE import_items
                 SET status = 'done', memories_created = ?3, memories_updated = ?4,
                     error = NULL, finished_at = ?5
                 WHERE job_id = ?1 AND source_id = ?2",
                params![job_id, source_id, memories_created, memories_updated, finished_at],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    pub(crate) fn mark_item_failed(
        &self,
        job_id: &str,
        source_id: &str,
        error: &str,
        finished_at: i64,
    ) -> Result<(), AppError> {
        self.connection()?
            .execute(
                "UPDATE import_items
                 SET status = 'failed', error = ?3, finished_at = ?4
                 WHERE job_id = ?1 AND source_id = ?2",
                params![job_id, source_id, error, finished_at],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    /// One "retry failed" at the end of a run, per the plan — failures rejoin
    /// the queue, their old error wiped so a stale message can't outlive it.
    pub(crate) fn requeue_failed(&self, job_id: &str) -> Result<i64, AppError> {
        let changed = self
            .connection()?
            .execute(
                "UPDATE import_items
                 SET status = 'pending', error = NULL, finished_at = NULL
                 WHERE job_id = ?1 AND status = 'failed'",
                [job_id],
            )
            .map_err(AppError::database)?;
        Ok(changed as i64)
    }

    pub(crate) fn set_job_status(
        &self,
        job_id: &str,
        status: JobStatus,
        error: Option<&str>,
        updated_at: i64,
    ) -> Result<(), AppError> {
        self.connection()?
            .execute(
                "UPDATE import_jobs
                 SET status = ?2, error = ?3, updated_at = ?4
                 WHERE id = ?1",
                params![job_id, status.as_str(), error, updated_at],
            )
            .map_err(AppError::database)?;
        Ok(())
    }

    /// A `running` row after a process start is a job the previous process
    /// died holding. Park it as `paused` so the UI offers resume instead of
    /// showing a run nobody is running.
    pub(crate) fn park_orphaned_jobs(&self, updated_at: i64) -> Result<i64, AppError> {
        let changed = self
            .connection()?
            .execute(
                "UPDATE import_jobs
                 SET status = 'paused', updated_at = ?1
                 WHERE status = 'running'",
                [updated_at],
            )
            .map_err(AppError::database)?;
        Ok(changed as i64)
    }

    pub(crate) fn snapshot(&self, job_id: &str) -> Result<Option<ImportJobSnapshot>, AppError> {
        let connection = self.connection()?;
        let job = connection
            .query_row(
                "SELECT id, companion_id, agent_ref, source, source_path, status,
                        error, include_claude_memories, created_at, updated_at
                 FROM import_jobs
                 WHERE id = ?1",
                [job_id],
                map_job,
            )
            .optional()
            .map_err(AppError::database)?;
        let Some(job) = job else { return Ok(None) };
        let snapshot = connection
            .query_row(
                "SELECT
                     COUNT(*) FILTER (WHERE status = 'pending'),
                     COUNT(*) FILTER (WHERE status = 'done'),
                     COUNT(*) FILTER (WHERE status = 'failed'),
                     COUNT(*) FILTER (WHERE status = 'skipped'),
                     COALESCE(SUM(memories_created), 0),
                     COALESCE(SUM(memories_updated), 0)
                 FROM import_items
                 WHERE job_id = ?1",
                [job_id],
                |row| {
                    Ok(ImportJobSnapshot {
                        job: job.clone(),
                        pending: row.get(0)?,
                        done: row.get(1)?,
                        failed: row.get(2)?,
                        skipped: row.get(3)?,
                        memories_created: row.get(4)?,
                        memories_updated: row.get(5)?,
                    })
                },
            )
            .map_err(AppError::database)?;
        Ok(Some(snapshot))
    }

    /// Jobs for the UI, newest first — the import history a companion's
    /// settings page can show.
    pub(crate) fn list_snapshots(&self) -> Result<Vec<ImportJobSnapshot>, AppError> {
        let ids: Vec<String> = {
            let connection = self.connection()?;
            let mut statement = connection
                .prepare("SELECT id FROM import_jobs ORDER BY created_at DESC, id DESC")
                .map_err(AppError::database)?;
            let ids = statement
                .query_map([], |row| row.get(0))
                .map_err(AppError::database)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(AppError::database)?;
            ids
        };
        ids.iter()
            .filter_map(|id| self.snapshot(id).transpose())
            .collect()
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| AppError::internal("the local database lock was poisoned"))
    }
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportJob> {
    Ok(ImportJob {
        id: row.get(0)?,
        companion_id: row.get(1)?,
        agent_ref: row.get(2)?,
        source: ImportSource::from_storage(&row.get::<_, String>(3)?),
        source_path: row.get(4)?,
        status: JobStatus::from_storage(&row.get::<_, String>(5)?),
        error: row.get(6)?,
        include_claude_memories: row.get::<_, i64>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ImportJob, ImportRepository, JobStatus, NewImportItem};
    use crate::{database, import::ImportSource};

    fn open_repository(tag: &str) -> (ImportRepository, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "companion-import-ledger-{tag}-{}.db",
            uuid::Uuid::new_v4()
        ));
        database::initialise(&path).expect("test database should initialise");
        (ImportRepository::open(&path).expect("repository should open"), path)
    }

    fn built_in_id(path: &std::path::Path) -> String {
        rusqlite::Connection::open(path)
            .expect("test database should open")
            .query_row("SELECT id FROM companions WHERE is_built_in = 1", [], |row| {
                row.get(0)
            })
            .expect("the built-in companion should exist")
    }

    fn seed_companion(path: &std::path::Path, id: &str) {
        rusqlite::Connection::open(path)
            .expect("test database should open")
            .execute(
                "INSERT INTO companions (
                     id, name, memory_agent_name, model_preference_mode, model_id,
                     is_built_in, created_at, updated_at, is_origin
                 ) VALUES (?1, ?2, ?3, 'inherit', NULL, 0, 0, 0, 0)",
                rusqlite::params![id, "Second", format!("companion-{id}")],
            )
            .expect("companion should insert");
    }

    fn job(id: &str, companion_id: &str) -> ImportJob {
        ImportJob {
            id: id.to_owned(),
            companion_id: companion_id.to_owned(),
            agent_ref: "agent-1".to_owned(),
            source: ImportSource::Claude,
            source_path: "/tmp/export.zip".to_owned(),
            status: JobStatus::Running,
            error: None,
            include_claude_memories: true,
            created_at: 1_000,
            updated_at: 1_000,
        }
    }

    fn item(source_id: &str, conversation_at: i64, source_updated: i64) -> NewImportItem {
        NewImportItem {
            source_id: source_id.to_owned(),
            title: format!("Chat {source_id}"),
            conversation_at,
            source_updated,
        }
    }

    #[test]
    fn the_queue_serves_oldest_conversations_first() {
        let (repository, path) = open_repository("queue-order");
        let companion = built_in_id(&path);
        repository
            .create_job(
                &job("job-1", &companion),
                &[item("newer", 2_000, 2_000), item("older", 1_000, 1_000)],
            )
            .expect("job should create");

        let first = repository.next_pending("job-1").expect("query works").expect("has pending");
        assert_eq!(first.source_id, "older");

        repository.mark_item_done("job-1", "older", 3, 1, 5_000).expect("marks");
        let second = repository.next_pending("job-1").expect("query works").expect("has pending");
        assert_eq!(second.source_id, "newer");

        fs::remove_file(path).ok();
    }

    /// The re-import story: a second drop of the export runs only what
    /// changed — for the same companion. A different companion starts fresh.
    #[test]
    fn a_second_drop_skips_what_this_companion_already_holds() {
        let (repository, path) = open_repository("dedupe");
        let companion = built_in_id(&path);
        repository
            .create_job(&job("job-1", &companion), &[item("conv-a", 1_000, 1_000)])
            .expect("first job creates");
        repository.mark_item_done("job-1", "conv-a", 2, 0, 5_000).expect("marks");

        // Same companion, same conversation unchanged + one that grew newer.
        repository
            .create_job(
                &job("job-2", &companion),
                &[item("conv-a", 1_000, 1_000), item("conv-a-grown", 1_000, 9_000)],
            )
            .expect("second job creates");
        let snapshot = repository.snapshot("job-2").expect("query works").expect("exists");
        assert_eq!(snapshot.skipped, 1, "the unchanged conversation is born skipped");
        assert_eq!(snapshot.pending, 1, "the changed one queues");

        // A different companion has none of this history.
        seed_companion(&path, "companion-2");
        repository
            .create_job(&job("job-3", "companion-2"), &[item("conv-a", 1_000, 1_000)])
            .expect("third job creates");
        let fresh = repository.snapshot("job-3").expect("query works").expect("exists");
        assert_eq!(fresh.pending, 1, "another companion imports the same history in full");

        fs::remove_file(path).ok();
    }

    /// An updated conversation must NOT be skipped: the export's
    /// `source_updated` outranks what the ledger finished with.
    #[test]
    fn an_updated_conversation_requeues_instead_of_skipping() {
        let (repository, path) = open_repository("updated");
        let companion = built_in_id(&path);
        repository
            .create_job(&job("job-1", &companion), &[item("conv-a", 1_000, 1_000)])
            .expect("first job creates");
        repository.mark_item_done("job-1", "conv-a", 1, 0, 5_000).expect("marks");

        repository
            .create_job(&job("job-2", &companion), &[item("conv-a", 1_000, 2_000)])
            .expect("second job creates");

        let snapshot = repository.snapshot("job-2").expect("query works").expect("exists");
        assert_eq!(snapshot.pending, 1, "the conversation grew since it was distilled");
        assert_eq!(snapshot.skipped, 0);

        fs::remove_file(path).ok();
    }

    #[test]
    fn failures_count_and_requeue_for_one_retry_pass() {
        let (repository, path) = open_repository("retry");
        let companion = built_in_id(&path);
        repository
            .create_job(
                &job("job-1", &companion),
                &[item("conv-a", 1_000, 1_000), item("conv-b", 2_000, 2_000)],
            )
            .expect("job creates");
        repository
            .mark_item_failed("job-1", "conv-a", "rate limited", 5_000)
            .expect("marks");
        repository.mark_item_done("job-1", "conv-b", 4, 2, 6_000).expect("marks");

        let snapshot = repository.snapshot("job-1").expect("query works").expect("exists");
        assert_eq!((snapshot.failed, snapshot.done, snapshot.pending), (1, 1, 0));
        assert_eq!(snapshot.memories_created, 4);
        assert_eq!(snapshot.memories_updated, 2);

        let requeued = repository.requeue_failed("job-1").expect("requeues");
        assert_eq!(requeued, 1);
        let next = repository.next_pending("job-1").expect("query works").expect("has pending");
        assert_eq!(next.source_id, "conv-a");

        fs::remove_file(path).ok();
    }

    /// The restart story: a job the previous process died holding is parked as
    /// paused, so the UI offers resume instead of rendering a phantom run.
    #[test]
    fn a_running_job_from_a_dead_process_parks_as_paused() {
        let (repository, path) = open_repository("park");
        let companion = built_in_id(&path);
        repository
            .create_job(&job("job-1", &companion), &[item("conv-a", 1_000, 1_000)])
            .expect("job creates");

        let parked = repository.park_orphaned_jobs(9_000).expect("parks");

        assert_eq!(parked, 1);
        let snapshot = repository.snapshot("job-1").expect("query works").expect("exists");
        assert_eq!(snapshot.job.status, JobStatus::Paused);

        fs::remove_file(path).ok();
    }
}
