use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

use serde::Serialize;

use crate::models::{AppError, AppResult};

/// A successful result always means the new bytes have been committed.
/// Directory-sync failure after rename is a durability warning, NOT a failed
/// save: reporting failure then would falsely imply the old bytes still exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomicWriteOutcome {
    pub directory_sync_confirmed: bool,
}

/// Write and flush a same-directory temporary file before atomic replacement.
/// Any Err occurs before replacement, leaving the previous destination intact.
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> AppResult<AtomicWriteOutcome> {
    atomic_replace_with(path, |file| file.write_all(bytes))
}

/// Stream a document/settings value directly to disk without buffering a JSON copy.
pub fn atomic_replace_json<T: Serialize>(path: &Path, value: &T) -> AppResult<AtomicWriteOutcome> {
    atomic_replace_with(path, |file| {
        serde_json::to_writer_pretty(file, value).map_err(io::Error::other)
    })
}

/// The writer must not modify the final destination itself. Temporary files are
/// private by default and are removed on all pre-commit failure paths.
pub fn atomic_replace_with<F>(path: &Path, writer: F) -> AppResult<AtomicWriteOutcome>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    if path.file_name().is_none() {
        return Err(AppError::new("invalidDestination", "Choose a file destination, not a directory."));
    }
    let directory = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    #[cfg(unix)]
    let directory_handle = File::open(directory).map_err(save_error)?;

    let mut temporary = tempfile::Builder::new()
        .prefix(".quicktranslator-")
        .tempfile_in(directory)
        .map_err(save_error)?;
    writer(temporary.as_file_mut()).map_err(save_error)?;
    temporary.as_file_mut().flush().map_err(save_error)?;
    temporary.as_file().sync_all().map_err(save_error)?;
    // tempfile uses same-filesystem rename, including replacement on Windows.
    temporary.persist(path).map_err(|failure| save_error(failure.error))?;

    #[cfg(unix)]
    let directory_sync_confirmed = directory_handle.sync_all().is_ok();
    #[cfg(not(unix))]
    let directory_sync_confirmed = false;

    Ok(AtomicWriteOutcome { directory_sync_confirmed })
}

fn save_error(error: io::Error) -> AppError {
    AppError::io("saveFailed", "The file could not be safely saved; previous contents were not replaced", &error)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn interrupted_writer_preserves_old_bytes_and_cleans_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("document.qtp");
        fs::write(&destination, b"previous saved document").unwrap();
        let result = atomic_replace_with(&destination, |file| {
            file.write_all(b"partial replacement")?;
            Err(io::Error::new(io::ErrorKind::WriteZero, "private content must not leak"))
        });
        let error = result.unwrap_err();
        assert!(!error.message.contains("private content"));
        assert_eq!(fs::read(&destination).unwrap(), b"previous saved document");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn rename_failure_does_not_remove_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("occupied");
        fs::create_dir(&destination).unwrap();
        let retained = destination.join("retained.txt");
        fs::write(&retained, b"keep me").unwrap();
        assert!(atomic_replace(&destination, b"replacement").is_err());
        assert_eq!(fs::read(&retained).unwrap(), b"keep me");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn unicode_json_replaces_existing_file_without_leftover_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("settings.json");
        fs::write(&destination, b"old").unwrap();
        let expected = serde_json::json!({ "text": "Tiếng Việt 日本語 🙂" });
        atomic_replace_json(&destination, &expected).unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
