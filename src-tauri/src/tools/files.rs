// The companion's file tools — five hands on ONE explicitly selected named
// folder per call. Every allowed root arrives canonical (the companion service
// stores it that way, chat re-canonicalises at submission), and every call
// re-proves containment before touching the disk: paths are relative-only,
// ".." never passes, and symlinks are resolved and checked so a link inside a
// folder cannot smuggle an operation out of it. No workspace grants = these
// tools are never declared; there is no fallback directory, ever.

use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) const LIST_FILES: &str = "list_files";
pub(crate) const READ_FILE: &str = "read_file";
pub(crate) const WRITE_FILE: &str = "write_file";
pub(crate) const EDIT_FILE: &str = "edit_file";
pub(crate) const DELETE_FILE: &str = "delete_file";

pub(crate) const FILE_TOOL_NAMES: [&str; 5] =
    [LIST_FILES, READ_FILE, WRITE_FILE, EDIT_FILE, DELETE_FILE];

/// A read is truncated at this many chars — same budget the other big
/// renders use, with an honest marker when the file continues past it.
const READ_RENDER_BUDGET_CHARS: usize = 24_000;
const LIST_MAX_ENTRIES: usize = 300;

/// Dispatch one file tool call against the workspace. Runs blocking file IO —
/// the caller wraps it in spawn_blocking.
pub(crate) fn execute(name: &str, arguments: &str, root: &Path) -> Result<String, String> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| format!("arguments were not valid JSON: {error}"))?;
    let text_field = |key: &str| -> Result<String, String> {
        parsed
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("a non-empty \"{key}\" argument is required"))
    };
    match name {
        LIST_FILES => {
            let path = parsed
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_owned();
            list(root, &path)
        }
        READ_FILE => read(root, &text_field("path")?),
        WRITE_FILE => {
            // Content may legitimately be empty — an empty file is a real file.
            let content = parsed
                .get("content")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "a \"content\" argument is required".to_owned())?;
            write(root, &text_field("path")?, content)
        }
        EDIT_FILE => edit(
            root,
            &text_field("path")?,
            &text_field("old_text")?,
            parsed
                .get("new_text")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "a \"new_text\" argument is required".to_owned())?,
        ),
        DELETE_FILE => delete(root, &text_field("path")?),
        other => Err(format!("unknown file tool \"{other}\"")),
    }
}

/// The whole security story lives here. A requested path must be relative,
/// carry no "..", and — once joined under the root — its deepest existing
/// ancestor must canonicalise back INSIDE the root, which is what defeats a
/// symlink pointing out of the workspace. Everything the tools touch goes
/// through this door.
fn resolve(root: &Path, requested: &str) -> Result<PathBuf, String> {
    let requested = requested.trim();
    let mut relative = PathBuf::new();
    for component in Path::new(requested).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(
                    "\"..\" is not allowed — give a path inside the workspace folder".to_owned(),
                )
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(
                    "give a path relative to the workspace folder, not an absolute one".to_owned(),
                )
            }
        }
    }
    let joined = root.join(&relative);

    let mut deepest_existing = joined.clone();
    while !deepest_existing.exists() {
        deepest_existing = match deepest_existing.parent() {
            Some(parent) => parent.to_path_buf(),
            None => return Err("the workspace folder has gone missing".to_owned()),
        };
    }
    // A broken symlink reports "does not exist" while still being a real link
    // a write would follow — refuse it rather than create a file at wherever
    // it points.
    if deepest_existing != joined && joined.symlink_metadata().is_ok() {
        return Err(format!(
            "\"{requested}\" is a symbolic link with no target — refused"
        ));
    }
    let canonical = fs::canonicalize(&deepest_existing)
        .map_err(|error| format!("could not resolve \"{requested}\": {error}"))?;
    if !canonical.starts_with(root) {
        return Err(format!(
            "\"{requested}\" points outside the workspace folder — refused"
        ));
    }
    Ok(joined)
}

fn list(root: &Path, path: &str) -> Result<String, String> {
    let target = resolve(root, path)?;
    if !target.is_dir() {
        return Err(format!(
            "\"{}\" is not a folder — use read_file for files",
            display_path(path)
        ));
    }
    let mut entries: Vec<(bool, String, u64)> = fs::read_dir(&target)
        .map_err(|error| format!("the folder could not be listed: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let metadata = entry.metadata().ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = metadata.map(|m| m.len()).unwrap_or(0);
            (is_dir, name, size)
        })
        .collect();
    if entries.is_empty() {
        return Ok(format!("\"{}\" is empty", display_path(path)));
    }
    entries.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));

    let total = entries.len();
    let rows: Vec<String> = entries
        .into_iter()
        .take(LIST_MAX_ENTRIES)
        .map(|(is_dir, name, size)| {
            if is_dir {
                format!("{name}/")
            } else {
                format!("{name} ({})", human_size(size))
            }
        })
        .collect();
    let mut rendered = format!(
        "{} entr{} in \"{}\" (folders first):\n{}",
        total,
        if total == 1 { "y" } else { "ies" },
        display_path(path),
        rows.join("\n")
    );
    if total > LIST_MAX_ENTRIES {
        rendered.push_str(&format!(
            "\n({} more entr{} withheld — list a subfolder to reach them)",
            total - LIST_MAX_ENTRIES,
            if total - LIST_MAX_ENTRIES == 1 { "y" } else { "ies" }
        ));
    }
    Ok(rendered)
}

fn read(root: &Path, path: &str) -> Result<String, String> {
    let target = resolve(root, path)?;
    if target.is_dir() {
        return Err(format!(
            "\"{path}\" is a folder — use list_files for folders"
        ));
    }
    let bytes =
        fs::read(&target).map_err(|error| format!("\"{path}\" could not be read: {error}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| format!("\"{path}\" is not a text file — it cannot be read as text"))?;
    if text.is_empty() {
        return Ok(format!("\"{path}\" is empty"));
    }
    if text.chars().count() > READ_RENDER_BUDGET_CHARS {
        let truncated: String = text.chars().take(READ_RENDER_BUDGET_CHARS).collect();
        return Ok(format!(
            "{truncated}\n\n(truncated — \"{path}\" continues past this point)"
        ));
    }
    Ok(text)
}

fn write(root: &Path, path: &str, content: &str) -> Result<String, String> {
    let target = resolve(root, path)?;
    if target.is_dir() {
        return Err(format!("\"{path}\" is a folder — it cannot be overwritten"));
    }
    let existed = target.exists();
    if let Some(parent) = target.parent() {
        // The parent chain is already proven contained by resolve().
        fs::create_dir_all(parent)
            .map_err(|error| format!("the folders above \"{path}\" could not be created: {error}"))?;
    }
    fs::write(&target, content)
        .map_err(|error| format!("\"{path}\" could not be written: {error}"))?;
    Ok(format!(
        "{} \"{path}\" ({})",
        if existed { "overwrote" } else { "wrote" },
        human_size(content.len() as u64)
    ))
}

fn edit(root: &Path, path: &str, old_text: &str, new_text: &str) -> Result<String, String> {
    let target = resolve(root, path)?;
    let text = fs::read_to_string(&target)
        .map_err(|error| format!("\"{path}\" could not be read: {error}"))?;
    let matches = text.matches(old_text).count();
    if matches == 0 {
        return Err(format!(
            "the old_text was not found in \"{path}\" — read the file and copy the exact text"
        ));
    }
    if matches > 1 {
        return Err(format!(
            "the old_text appears {matches} times in \"{path}\" — include more surrounding \
             lines so it matches exactly once"
        ));
    }
    let edited = text.replacen(old_text, new_text, 1);
    fs::write(&target, edited)
        .map_err(|error| format!("\"{path}\" could not be written: {error}"))?;
    Ok(format!("edited \"{path}\" — the replacement is in place"))
}

fn delete(root: &Path, path: &str) -> Result<String, String> {
    let target = resolve(root, path)?;
    let canonical_target = fs::canonicalize(&target)
        .map_err(|error| format!("\"{path}\" could not be found: {error}"))?;
    if canonical_target == root {
        return Err("the workspace folder itself cannot be deleted".to_owned());
    }
    let metadata = target
        .symlink_metadata()
        .map_err(|error| format!("\"{path}\" could not be found: {error}"))?;
    if metadata.is_dir() {
        // Only an EMPTY folder goes — a recursive delete is a bigger hammer
        // than a chat turn should swing.
        fs::remove_dir(&target).map_err(|error| {
            format!(
                "\"{path}\" could not be deleted (only empty folders can be): {error}"
            )
        })?;
        Ok(format!("deleted the empty folder \"{path}\""))
    } else {
        fs::remove_file(&target)
            .map_err(|error| format!("\"{path}\" could not be deleted: {error}"))?;
        Ok(format!("deleted \"{path}\""))
    }
}

/// The workspace root itself reads better with a name than as "".
fn display_path(path: &str) -> &str {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        "the workspace folder"
    } else {
        trimmed
    }
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{kb:.1} KB");
    }
    format!("{:.1} MB", kb / 1024.0)
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use uuid::Uuid;

    use super::{delete, edit, execute, list, read, resolve, write};

    /// A real folder on disk — containment can only be proven against a
    /// filesystem that actually resolves symlinks.
    struct ScratchWorkspace {
        root: PathBuf,
        /// A sibling OUTSIDE the workspace, for escape attempts.
        outside: PathBuf,
    }

    impl ScratchWorkspace {
        fn new() -> Self {
            let base = env::temp_dir().join(format!("companion-files-test-{}", Uuid::new_v4()));
            let root = base.join("workspace");
            let outside = base.join("outside");
            fs::create_dir_all(&root).expect("the workspace should be created");
            fs::create_dir_all(&outside).expect("the outside folder should be created");
            let root = fs::canonicalize(&root).expect("the workspace should canonicalise");
            Self { root, outside }
        }
    }

    impl Drop for ScratchWorkspace {
        fn drop(&mut self) {
            if let Some(base) = self.root.parent() {
                let _ = fs::remove_dir_all(base);
            }
        }
    }

    #[test]
    fn paths_that_try_to_escape_are_refused() {
        let workspace = ScratchWorkspace::new();
        for escape in [
            "../outside/secret.txt",
            "notes/../../outside/secret.txt",
            "/etc/passwd",
            "..",
        ] {
            assert!(
                resolve(&workspace.root, escape).is_err(),
                "\"{escape}\" must be refused"
            );
        }
        // Plain relative paths resolve, including into folders that do not
        // exist yet — that is what lets write_file create them.
        assert!(resolve(&workspace.root, "notes/today.md").is_ok());
        assert!(resolve(&workspace.root, "./a/./b.txt").is_ok());
        assert!(resolve(&workspace.root, "").is_ok(), "the root itself resolves");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_outside_the_workspace_is_refused() {
        let workspace = ScratchWorkspace::new();
        fs::write(workspace.outside.join("secret.txt"), "outside")
            .expect("the outside file should seed");
        std::os::unix::fs::symlink(&workspace.outside, workspace.root.join("sneaky"))
            .expect("the symlink should be created");

        // Through the link as a folder, and the link itself as a target.
        assert!(resolve(&workspace.root, "sneaky/secret.txt").is_err());
        assert!(resolve(&workspace.root, "sneaky/new.txt").is_err());
        assert!(read(&workspace.root, "sneaky/secret.txt").is_err());
        assert!(write(&workspace.root, "sneaky/planted.txt", "x").is_err());
        assert!(
            !workspace.outside.join("planted.txt").exists(),
            "nothing may land outside the workspace"
        );

        // A broken symlink "does not exist", but a write would follow it out.
        std::os::unix::fs::symlink(
            workspace.outside.join("not-yet.txt"),
            workspace.root.join("dangling.txt"),
        )
        .expect("the dangling symlink should be created");
        assert!(write(&workspace.root, "dangling.txt", "x").is_err());
        assert!(!workspace.outside.join("not-yet.txt").exists());
    }

    #[test]
    fn the_five_hands_work_inside_the_workspace() {
        let workspace = ScratchWorkspace::new();

        // write — creating the folders on the way.
        let wrote = write(&workspace.root, "notes/today.md", "ship the workspace")
            .expect("the write should land");
        assert!(wrote.starts_with("wrote"));
        let overwrote = write(&workspace.root, "notes/today.md", "ship the workspace tools")
            .expect("the overwrite should land");
        assert!(overwrote.starts_with("overwrote"));

        // read — exactly what was written.
        assert_eq!(
            read(&workspace.root, "notes/today.md").expect("the read should land"),
            "ship the workspace tools"
        );

        // edit — unique match replaced, absent and ambiguous both refused.
        edit(&workspace.root, "notes/today.md", "the workspace tools", "it")
            .expect("the edit should land");
        assert_eq!(
            read(&workspace.root, "notes/today.md").expect("the read should land"),
            "ship it"
        );
        assert!(edit(&workspace.root, "notes/today.md", "not there", "x").is_err());
        write(&workspace.root, "notes/twice.txt", "aba aba").expect("the seed should land");
        assert!(
            edit(&workspace.root, "notes/twice.txt", "aba", "x").is_err(),
            "an ambiguous match must be refused"
        );

        // list — folders first, sizes on files.
        let listing = list(&workspace.root, "").expect("the root should list");
        assert!(listing.contains("notes/"));
        let notes = list(&workspace.root, "notes").expect("the folder should list");
        assert!(notes.contains("today.md (7 B)"));

        // delete — files yes, the root never, non-empty folders never.
        delete(&workspace.root, "notes/twice.txt").expect("the delete should land");
        assert!(delete(&workspace.root, "").is_err(), "the root must survive");
        assert!(delete(&workspace.root, "notes").is_err(), "non-empty folder refused");
        delete(&workspace.root, "notes/today.md").expect("the file should go");
        delete(&workspace.root, "notes").expect("the now-empty folder should go");
    }

    #[test]
    fn execute_parses_arguments_and_requires_what_each_tool_needs() {
        let workspace = ScratchWorkspace::new();

        assert!(execute(
            "write_file",
            r#"{"path":"a.txt","content":"hello"}"#,
            &workspace.root
        )
        .is_ok());
        // Empty content is a real file, not a missing argument.
        assert!(execute(
            "write_file",
            r#"{"path":"empty.txt","content":""}"#,
            &workspace.root
        )
        .is_ok());
        assert!(execute("write_file", r#"{"path":"a.txt"}"#, &workspace.root).is_err());
        assert_eq!(
            execute("read_file", r#"{"path":"a.txt"}"#, &workspace.root),
            Ok("hello".to_owned())
        );
        assert!(execute("read_file", r#"{}"#, &workspace.root).is_err());
        assert!(execute(
            "edit_file",
            r#"{"path":"a.txt","old_text":"hello","new_text":"hi"}"#,
            &workspace.root
        )
        .is_ok());
        assert!(execute("list_files", r#"{}"#, &workspace.root).is_ok());
        assert!(execute("delete_file", r#"{"path":"a.txt"}"#, &workspace.root).is_ok());
        assert!(execute("not_a_tool", r#"{}"#, &workspace.root).is_err());
    }
}
