//! A companion's face — the picture the user gives it, on disk.
//!
//! ⚑ WHERE THESE LIVE, AND WHY IT IS NOT A FREE CHOICE. Avatars sit in
//! `avatars/` NEXT TO `companion.db`, which puts them at
//! `~/.semantix/companion/avatars/` — the global Semantix folder, the same
//! anchor the database earned the hard way. That path is derived from the
//! database's own parent rather than resolved a second time from `$HOME`, so
//! the two can never drift apart: whatever fixed path the database is opened
//! at, the faces are beside it. See `resolve_database_path` in `lib.rs` for
//! the data-loss bug that made `$HOME` non-negotiable — Tauri's
//! `app_local_data_dir()` carries the VSCode snap REVISION NUMBER, so every
//! IDE update would otherwise hand the app an empty avatar folder.
//!
//! ⚑ AND WHY NOT IN THE DATABASE. `companion.db` is already ~200 MB of
//! conversation. Photographs in a row would ride every backup, every VACUUM,
//! and cross the IPC boundary on every roster read. The database keeps only
//! the FILE NAME (`<companion-id>.png`); the bytes are a file.
//!
//! A file is named for the companion that owns it, so ownership needs no
//! bookkeeping: deleting a companion deletes `<id>.*`, and there is no way to
//! orphan a picture or collide two.

use std::{
    fs,
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::app_error::AppError;

/// Big enough for a photograph straight off a phone, small enough that a
/// roster of them stays a sane payload. Checked against the file on disk
/// BEFORE anything is read into memory.
const MAX_AVATAR_BYTES: u64 = 4 * 1024 * 1024;

/// The formats a webview will actually render, each recognised by its magic
/// bytes rather than its file extension.
///
/// ⚑ THE EXTENSION IS NOT EVIDENCE. It is user-supplied text on a file we are
/// about to hand to a browser engine as an image. `sniff` reads the header
/// instead, and the extension we store is the one the CONTENT earned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvatarKind {
    Png,
    Jpeg,
    Gif,
    WebP,
}

impl AvatarKind {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::WebP => "webp",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
        }
    }

    fn from_extension(extension: &str) -> Option<Self> {
        match extension {
            "png" => Some(Self::Png),
            "jpg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }
}

/// Identify an image by its header. Anything unrecognised is refused rather
/// than stored on a guess.
fn sniff(bytes: &[u8]) -> Option<AvatarKind> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(AvatarKind::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(AvatarKind::Jpeg);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(AvatarKind::Gif);
    }
    // RIFF....WEBP — the four size bytes at offset 4 sit between the two tags.
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(AvatarKind::WebP);
    }
    None
}

/// `~/.semantix/companion/avatars`, derived from the database beside it.
///
/// A database path with no parent means an in-memory or bare-filename handle,
/// which only the tests produce; `avatars/` relative to the working directory
/// is the honest reading of it and keeps this function total.
pub(crate) fn directory(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("avatars")
}

/// Copy the user's chosen image in, and return the file name to record.
///
/// The write is by companion id, so setting a new face REPLACES the old one
/// including a change of format — every `<id>.*` is cleared first, or a
/// switch from `a.png` to `a.jpg` would leave the stale PNG behind to be
/// found by a later reader.
pub(crate) fn store(
    directory: &Path,
    companion_id: &str,
    source: &Path,
) -> Result<String, AppError> {
    let metadata = fs::metadata(source)
        .map_err(|_| AppError::validation("That image could not be found on this machine."))?;
    if !metadata.is_file() {
        return Err(AppError::validation("An avatar must be a file, not a folder."));
    }
    if metadata.len() > MAX_AVATAR_BYTES {
        return Err(AppError::validation(format!(
            "An avatar must be {} MB or smaller.",
            MAX_AVATAR_BYTES / (1024 * 1024)
        )));
    }

    let bytes = fs::read(source)
        .map_err(|_| AppError::validation("That image could not be read."))?;
    let kind = sniff(&bytes).ok_or_else(|| {
        AppError::validation("An avatar must be a PNG, JPEG, GIF, or WebP image.")
    })?;

    fs::create_dir_all(directory).map_err(|error| {
        AppError::internal(format!("The avatar folder could not be created: {error}"))
    })?;
    remove(directory, companion_id)?;

    let file_name = format!("{companion_id}.{}", kind.extension());
    fs::write(directory.join(&file_name), &bytes).map_err(|error| {
        AppError::internal(format!("The avatar could not be saved: {error}"))
    })?;
    Ok(file_name)
}

/// Drop every picture belonging to this companion, whatever its format.
///
/// Called both when a user clears their avatar and when the companion itself
/// is deleted — a face never outlives the one who wore it. A missing folder
/// is success: there is nothing to remove.
pub(crate) fn remove(directory: &Path, companion_id: &str) -> Result<(), AppError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_stem().and_then(|stem| stem.to_str()) == Some(companion_id) {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// The stored file as a `data:` URL the webview can render directly.
///
/// ⚑ WHY A DATA URL AND NOT A FILE URL. A webview cannot read an arbitrary
/// path, so serving these would mean enabling Tauri's asset protocol and
/// scoping it to this folder. That is the better answer once a user has many
/// companions — the webview would cache each face instead of re-encoding it
/// per roster read. It is deliberately NOT the answer yet: the payload is one
/// small image per companion, and inlining removes a whole class of staleness
/// bug (an asset URL keyed on a path that does not change when the picture
/// behind it does). The files are already on disk in the right place, so that
/// upgrade is a change to this one function and the frontend, never a
/// migration.
///
/// A recorded name whose file has vanished reads as "no avatar" rather than an
/// error: the picture is a decoration, and a companion missing one still works.
pub(crate) fn read_data_url(directory: &Path, file_name: &str) -> Option<String> {
    let extension = Path::new(file_name).extension()?.to_str()?;
    let kind = AvatarKind::from_extension(extension)?;
    let bytes = fs::read(directory.join(file_name)).ok()?;
    Some(format!(
        "data:{};base64,{}",
        kind.mime(),
        STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use uuid::Uuid;

    use super::{directory, read_data_url, remove, store, MAX_AVATAR_BYTES};

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0];

    fn scratch() -> std::path::PathBuf {
        let path = env::temp_dir().join(format!("companion-avatars-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).expect("scratch directory should be created");
        path
    }

    #[test]
    fn avatars_sit_beside_the_database() {
        assert_eq!(
            directory(std::path::Path::new("/home/someone/.semantix/companion/companion.db")),
            std::path::Path::new("/home/someone/.semantix/companion/avatars"),
        );
    }

    #[test]
    fn an_image_is_stored_under_the_companions_own_id() {
        let root = scratch();
        let source = root.join("picked.png");
        fs::write(&source, PNG).expect("source should be written");

        let file_name = store(&root.join("avatars"), "companion-a", &source)
            .expect("a real png should store");

        assert_eq!(file_name, "companion-a.png");
        assert!(root.join("avatars").join("companion-a.png").is_file());
    }

    #[test]
    fn a_new_format_does_not_leave_the_old_file_behind() {
        let root = scratch();
        let avatars = root.join("avatars");
        let png = root.join("first.png");
        let jpeg = root.join("second.jpg");
        fs::write(&png, PNG).expect("png should be written");
        fs::write(&jpeg, JPEG).expect("jpeg should be written");

        store(&avatars, "companion-a", &png).expect("png should store");
        let file_name = store(&avatars, "companion-a", &jpeg).expect("jpeg should store");

        assert_eq!(file_name, "companion-a.jpg");
        assert!(!avatars.join("companion-a.png").exists());
        assert!(avatars.join("companion-a.jpg").is_file());
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused_however_it_is_named() {
        let root = scratch();
        let source = root.join("trojan.png");
        fs::write(&source, b"<html>not an image at all</html>").expect("source should be written");

        let error = store(&root.join("avatars"), "companion-a", &source)
            .expect_err("content, not the extension, decides");

        assert!(error.to_string().contains("PNG, JPEG, GIF, or WebP"));
    }

    #[test]
    fn an_oversized_image_is_refused_before_it_is_read() {
        let root = scratch();
        let source = root.join("huge.png");
        let mut bytes = PNG.to_vec();
        bytes.resize((MAX_AVATAR_BYTES + 1) as usize, 0);
        fs::write(&source, &bytes).expect("source should be written");

        let error = store(&root.join("avatars"), "companion-a", &source)
            .expect_err("an oversized file should be refused");

        assert!(error.to_string().contains("4 MB or smaller"));
    }

    #[test]
    fn removing_takes_every_format_and_leaves_other_companions_alone() {
        let root = scratch();
        let avatars = root.join("avatars");
        let source = root.join("picked.png");
        fs::write(&source, PNG).expect("source should be written");
        store(&avatars, "companion-a", &source).expect("a should store");
        store(&avatars, "companion-b", &source).expect("b should store");

        remove(&avatars, "companion-a").expect("removal should succeed");

        assert!(!avatars.join("companion-a.png").exists());
        assert!(avatars.join("companion-b.png").is_file());
    }

    #[test]
    fn a_recorded_name_whose_file_is_gone_reads_as_no_avatar() {
        let root = scratch();
        assert!(read_data_url(&root, "companion-a.png").is_none());
    }

    #[test]
    fn a_stored_image_reads_back_as_a_data_url_of_its_own_type() {
        let root = scratch();
        let avatars = root.join("avatars");
        let source = root.join("picked.jpg");
        fs::write(&source, JPEG).expect("source should be written");
        let file_name = store(&avatars, "companion-a", &source).expect("jpeg should store");

        let url = read_data_url(&avatars, &file_name).expect("a stored file should read back");

        assert!(url.starts_with("data:image/jpeg;base64,"));
    }
}
