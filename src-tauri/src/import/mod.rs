// History import — a Claude.ai or ChatGPT export read into normalized
// conversations the memory rail can distill, one at a time, the way /sleep
// already distills a live one.
//
// The user drops whatever they have — the zip as it downloaded, the folder it
// unzipped into, or a bare conversations.json — and the SOURCE IS SNIFFED from
// the JSON itself, never asked: a Claude conversation carries `chat_messages`,
// a ChatGPT one carries `mapping`. File names only decide where to look
// (`conversations.json`, or the sharded `conversations-NNN.json` of newer
// ChatGPT exports); they are not trusted to say which product exported them.
//
// Everything here is local parsing. Nothing leaves the machine until the
// import loop later hands conversations to the organ on the user's own key.

mod chatgpt;
mod claude;
pub(crate) mod repository;
pub(crate) mod worker;

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ImportSource {
    Claude,
    ChatGpt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TurnRole {
    User,
    Assistant,
}

pub(crate) struct ImportTurn {
    pub(crate) role: TurnRole,
    pub(crate) text: String,
    /// Which model spoke an assistant turn, when the export records it —
    /// ChatGPT stamps `metadata.model_slug` per message; Claude exports carry
    /// no model at all. The distiller ignores this; the style harvest filters
    /// on it ("give me the exchanges where gpt-4o itself replied").
    pub(crate) model_slug: Option<String>,
    /// When this turn was spoken, ms epoch. 0 = the export didn't say; the
    /// conversation's own stamp is the fallback.
    pub(crate) created_at_ms: i64,
}

/// One conversation in the export, already linearized and stripped to speech.
pub(crate) struct ImportedConversation {
    /// The export's own id — the re-import dedupe key, stable across drops.
    pub(crate) source_id: String,
    pub(crate) title: String,
    pub(crate) created_at_ms: i64,
    pub(crate) updated_at_ms: i64,
    pub(crate) turns: Vec<ImportTurn>,
}

pub(crate) struct ParsedExport {
    pub(crate) source: ImportSource,
    /// Oldest first — the order the import loop feeds the distiller, so eras
    /// accrue forward and name-reuse dedupe works with time, not against it.
    pub(crate) conversations: Vec<ImportedConversation>,
    pub(crate) empty_skipped: usize,
    /// Claude only: the markdown blobs of memories.json — the one part of an
    /// export that is already memory-shaped.
    pub(crate) claude_memories: Vec<String>,
}

const NOT_AN_EXPORT: &str = "That doesn't look like a Claude or ChatGPT export — \
it should contain a conversations.json file. Drop the .zip you downloaded, or the \
folder it unzipped into.";

/// Where the export's files live: still zipped, unzipped to a folder, or the
/// one JSON file on its own. Same questions answered either way: which
/// conversation files exist, and what bytes does one hold.
enum ExportContainer {
    Zip(zip::ZipArchive<std::fs::File>),
    Dir(PathBuf),
    File(PathBuf),
}

impl ExportContainer {
    fn open(path: &Path) -> Result<Self, String> {
        if path.is_dir() {
            return Ok(Self::Dir(path.to_owned()));
        }
        if !path.is_file() {
            return Err("That file or folder could not be found.".to_owned());
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("zip") => {
                let file = std::fs::File::open(path)
                    .map_err(|error| format!("The export could not be opened: {error}"))?;
                let archive = zip::ZipArchive::new(file)
                    .map_err(|error| format!("The zip could not be read: {error}"))?;
                Ok(Self::Zip(archive))
            }
            Some("json") => Ok(Self::File(path.to_owned())),
            _ => Err(NOT_AN_EXPORT.to_owned()),
        }
    }

    /// Names of the conversation files, sorted — so ChatGPT's zero-padded
    /// shards arrive in order. Entries may sit under a subfolder (someone
    /// re-zipped, or picked the folder above the unzip); only the final path
    /// component is judged.
    fn conversation_files(&mut self) -> Result<Vec<String>, String> {
        let is_conversations = |name: &str| {
            let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
            base == "conversations.json"
                || (base.starts_with("conversations-") && base.ends_with(".json"))
        };
        let mut names = match self {
            Self::Zip(archive) => archive
                .file_names()
                .filter(|name| is_conversations(name))
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Self::Dir(dir) => std::fs::read_dir(&dir)
                .map_err(|error| format!("The folder could not be read: {error}"))?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| is_conversations(name))
                .collect(),
            Self::File(path) => vec![path.to_string_lossy().into_owned()],
        };
        names.sort();
        if names.is_empty() {
            return Err(NOT_AN_EXPORT.to_owned());
        }
        Ok(names)
    }

    fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        match self {
            Self::Zip(archive) => {
                let mut entry = archive
                    .by_name(name)
                    .map_err(|error| format!("The export entry {name} could not be opened: {error}"))?;
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("The export entry {name} could not be read: {error}"))?;
                Ok(bytes)
            }
            Self::Dir(dir) => std::fs::read(dir.join(name))
                .map_err(|error| format!("{name} could not be read: {error}")),
            Self::File(path) => std::fs::read(&path)
                .map_err(|error| format!("The file could not be read: {error}")),
        }
    }

    /// The optional extra beside the conversations — present only in Claude
    /// exports, and only when found under the same roof.
    fn read_optional(&mut self, name: &str) -> Option<Vec<u8>> {
        match self {
            Self::Zip(archive) => {
                let full = archive
                    .file_names()
                    .find(|entry| {
                        entry.rsplit(['/', '\\']).next().unwrap_or(entry) == name
                    })
                    .map(str::to_owned)?;
                self.read(&full).ok()
            }
            Self::Dir(dir) => std::fs::read(dir.join(name)).ok(),
            Self::File(_) => None,
        }
    }
}

/// Which product wrote this export. Judged on the first 256KB of the first
/// conversation file: the key names are structural, present from the very
/// first conversation object, and no rename survives an export format that
/// third-party tools also parse.
fn sniff_source(head: &[u8]) -> Result<ImportSource, String> {
    let head = String::from_utf8_lossy(head);
    let position = |needle: &str| head.find(needle);
    match (position("\"chat_messages\""), position("\"mapping\"")) {
        (Some(_), None) => Ok(ImportSource::Claude),
        (None, Some(_)) => Ok(ImportSource::ChatGpt),
        // Both present in one object would mean a format merge nobody shipped;
        // trust whichever speaks first.
        (Some(claude), Some(gpt)) if claude < gpt => Ok(ImportSource::Claude),
        (Some(_), Some(_)) => Ok(ImportSource::ChatGpt),
        (None, None) => Err(
            "The conversations file doesn't match a Claude or ChatGPT export. \
             If the export looks empty, there may be nothing to import yet."
                .to_owned(),
        ),
    }
}

/// Read and parse a whole export from a zip, folder, or bare JSON path.
pub(crate) fn parse_export(path: &Path) -> Result<ParsedExport, String> {
    let mut container = ExportContainer::open(path)?;
    let files = container.conversation_files()?;

    let first = container.read(&files[0])?;
    let source = sniff_source(&first[..first.len().min(256 * 1024)])?;

    let mut conversations = Vec::new();
    let mut empty_skipped = 0usize;
    for (index, name) in files.iter().enumerate() {
        let bytes = if index == 0 { first.clone() } else { container.read(name)? };
        let (mut parsed, skipped) = match source {
            ImportSource::Claude => claude::parse_conversations(&bytes)?,
            ImportSource::ChatGpt => chatgpt::parse_conversations(&bytes)?,
        };
        conversations.append(&mut parsed);
        empty_skipped += skipped;
    }
    conversations.sort_by_key(|conversation| conversation.created_at_ms);

    // memories.json rides along when Claude packed one; a malformed file mutes
    // the bonus, never the import.
    let claude_memories = match source {
        ImportSource::Claude => container
            .read_optional("memories.json")
            .and_then(|bytes| match claude::parse_memories(&bytes) {
                Ok(memories) => Some(memories),
                Err(error) => {
                    eprintln!("[import] memories.json skipped: {error}");
                    None
                }
            })
            .unwrap_or_default(),
        ImportSource::ChatGpt => Vec::new(),
    };

    Ok(ParsedExport { source, conversations, empty_skipped, claude_memories })
}

/// What the wizard's preview shows before anything is imported: "Found 845
/// conversations, July 2023 → July 2026."
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportInspection {
    source: ImportSource,
    conversations: usize,
    total_turns: usize,
    empty_skipped: usize,
    earliest_ms: Option<i64>,
    latest_ms: Option<i64>,
    claude_memories: usize,
}

#[tauri::command]
pub(crate) async fn inspect_import_source(path: String) -> Result<ImportInspection, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let export = parse_export(Path::new(&path))?;
        let dated = || export.conversations.iter().filter(|c| c.created_at_ms > 0);
        Ok(ImportInspection {
            source: export.source,
            conversations: export.conversations.len(),
            total_turns: export.conversations.iter().map(|c| c.turns.len()).sum(),
            empty_skipped: export.empty_skipped,
            earliest_ms: dated().map(|c| c.created_at_ms).min(),
            latest_ms: dated()
                .map(|c| c.updated_at_ms.max(c.created_at_ms))
                .max(),
            claude_memories: export.claude_memories.len(),
        })
    })
    .await
    .map_err(|error| format!("Import inspection failed: {error}"))?
}

/// UTC ISO-8601 → epoch milliseconds, for Claude's `created_at` stamps.
/// Hand-rolled rather than a chrono dependency: the export speaks one dialect
/// ("2023-07-19T23:53:17.167873Z", occasionally "+00:00"), and a stamp that
/// doesn't parse costs an ordering hint, not a conversation.
pub(super) fn iso_to_epoch_ms(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    let bytes = stamp.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || (bytes[10] != b'T' && bytes[10] != b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| stamp.get(range)?.parse::<i64>().ok();
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut rest = &stamp[19..];
    let mut millis = 0i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        // Only the first three fractional digits are milliseconds; a shorter
        // fraction scales up (".5" is 500ms, not 5).
        let head = &fraction[..digits.min(3)];
        millis = head.parse::<i64>().ok()? * 10i64.pow(3 - head.len() as u32);
        rest = &fraction[digits..];
    }
    let offset_minutes = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = match rest.get(4..6) {
                Some(m) if rest.as_bytes().get(3) == Some(&b':') => m.parse::<i64>().ok()?,
                _ => 0,
            };
            sign * (hours * 60 + minutes)
        }
    };

    let seconds = days_from_civil(year, month, day) * 86_400
        + hour * 3_600
        + minute * 60
        + second
        - offset_minutes * 60;
    Some(seconds * 1_000 + millis)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Hinnant's
/// `days_from_civil`), the standard branchless civil-calendar algorithm.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::{iso_to_epoch_ms, parse_export, sniff_source, ImportSource};

    // Reference values computed independently (Python datetime, UTC).
    #[test]
    fn iso_stamps_parse_to_the_epoch() {
        assert_eq!(iso_to_epoch_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            iso_to_epoch_ms("2023-07-19T23:53:17.167873Z"),
            Some(1_689_810_797_167)
        );
        assert_eq!(
            iso_to_epoch_ms("2026-01-02T10:00:00.500Z"),
            Some(1_767_348_000_500)
        );
        assert_eq!(
            iso_to_epoch_ms("2024-03-10T12:00:00+03:00"),
            Some(1_710_061_200_000),
            "an offset stamp lands on the same UTC instant"
        );
        assert_eq!(iso_to_epoch_ms("not a date"), None);
        assert_eq!(iso_to_epoch_ms("2024-13-01T00:00:00Z"), None);
    }

    #[test]
    fn the_source_is_sniffed_from_structure_not_file_names() {
        assert_eq!(
            sniff_source(br#"[{"uuid": "x", "chat_messages": []}]"#),
            Ok(ImportSource::Claude)
        );
        assert_eq!(
            sniff_source(br#"[{"title": "x", "mapping": {}}]"#),
            Ok(ImportSource::ChatGpt)
        );
        assert!(sniff_source(b"[]").is_err(), "an empty export names no product");
    }

    /// The unpacked-folder path, end to end: Moti's note that some users will
    /// have the zip already unzipped.
    #[test]
    fn an_unpacked_folder_imports_like_the_zip() {
        let dir = std::env::temp_dir().join(format!("companion-import-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("conversations.json"),
            r#"[{"uuid": "c1", "name": "Chat", "created_at": "2024-01-01T00:00:00Z",
                 "updated_at": "2024-01-01T00:00:00Z",
                 "chat_messages": [{"sender": "human", "text": "hi", "content": []}]}]"#,
        )
        .expect("conversations");
        std::fs::write(dir.join("memories.json"), r#"[{"conversations_memory": "facts"}]"#)
            .expect("memories");

        let export = parse_export(&dir).expect("parses");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(export.source, ImportSource::Claude);
        assert_eq!(export.conversations.len(), 1);
        assert_eq!(export.claude_memories, vec!["facts".to_owned()]);
    }

    // === The real-corpus smoke tests — run by hand, never in CI ===
    //
    //   SEMANTIX_CLAUDE_EXPORT=/path/to/export.zip \
    //   cargo test --release real_ -- --ignored --nocapture
    //
    // They prove the parsers against actual exports, which no fixture can.

    fn smoke(env_key: &str) {
        let Ok(path) = std::env::var(env_key) else {
            panic!("set {env_key} to the export path");
        };
        let started = std::time::Instant::now();
        let export = parse_export(std::path::Path::new(&path)).expect("real export parses");
        let year = |ms: i64| 1970 + ms / 31_556_952_000;
        let dated: Vec<i64> = export
            .conversations
            .iter()
            .map(|c| c.created_at_ms)
            .filter(|&ms| ms > 0)
            .collect();
        let biggest = export.conversations.iter().map(|c| c.turns.len()).max().unwrap_or(0);
        println!(
            "source={:?} conversations={} empty_skipped={} total_turns={} biggest_turns={} \
             range={}..{} claude_memories={} elapsed={:?}",
            export.source,
            export.conversations.len(),
            export.empty_skipped,
            export.conversations.iter().map(|c| c.turns.len()).sum::<usize>(),
            biggest,
            dated.iter().min().map(|&ms| year(ms)).unwrap_or(0),
            dated.iter().max().map(|&ms| year(ms)).unwrap_or(0),
            export.claude_memories.len(),
            started.elapsed(),
        );
        assert!(!export.conversations.is_empty());
    }

    #[test]
    #[ignore]
    fn real_claude_export_smoke() {
        smoke("SEMANTIX_CLAUDE_EXPORT");
    }

    #[test]
    #[ignore]
    fn real_chatgpt_export_smoke() {
        smoke("SEMANTIX_GPT_EXPORT");
    }
}
