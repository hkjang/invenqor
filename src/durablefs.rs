//! Writes that either land whole or not at all.
//!
//! Both the state store and the updater need the same thing: replace a file
//! without ever leaving a half-written one behind, and survive a power loss
//! between the write and the rename. They had a copy each, and the copies were
//! not equal - the state store named its temporary file after the process id,
//! did not remove it when a write failed, and did not flush the directory.
//!
//! The process id was the one that bit. A temporary left behind by a process
//! that died between creating it and renaming it blocks every later write to
//! that path from any process that is later given the same id - which in a
//! container, where ids start at 1 and are reused, is the next run:
//!
//!   create /var/lib/invenqor-agent/last-heartbeat.tmp-1: File exists
//!
//! A name that cannot collide costs nothing and removes the whole class.

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Writes `bytes` to `path`, replacing whatever was there.
///
/// The temporary is named for this write, not for this process, so a temporary
/// left behind by a dead process can never block a later one.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut file = crate::platform::create_private_file(&temporary)?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    drop(file);
    replace_file(&temporary, path)?;
    sync_directory(path.parent().context("path has no parent")?)
}

/// Flushes the directory entry so a rename survives a power loss. Windows has no
/// equivalent call for a directory handle opened this way, and NTFS metadata
/// journaling covers the same ground, so it is a no-op there.
pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Moves `from` onto `to`, retrying briefly.
///
/// On Windows a file that was just written is routinely held open for a moment by
/// a virus scanner, and the rename fails with a sharing violation. Failing the
/// update for that would leave a host on the old version until someone noticed,
/// so the rename is retried for a few seconds before it is called an error.
pub fn replace_file(from: &Path, to: &Path) -> Result<()> {
    // Only Windows has a reason to wait. There a file that was just written is
    // routinely held open for a moment by an indexer or a virus scanner, and the
    // rename fails until the handle closes - so retrying is the difference
    // between a write that works and one that does not.
    //
    // rename(2) does not care about open handles, so on every other platform a
    // failure is EXDEV, ENOSPC, EACCES or EROFS. None of those clear up in five
    // seconds. Waiting anyway would delay the state store's writes - which is
    // new: they used to go straight to fs::rename before these two
    // implementations were merged - and then report a cause that cannot be the
    // cause, to a reader who is already looking for the real one.
    let attempts = if cfg!(windows) { 20 } else { 1 };
    let mut last = None;
    for attempt in 0..attempts {
        match rename_replacing(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < attempts - 1 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    let error = last.expect("at least one attempt");
    if cfg!(windows) {
        return Err(error).with_context(|| {
            format!(
                "replace {} - the file is held open by another process",
                to.display()
            )
        });
    }
    Err(error).with_context(|| format!("replace {}", to.display()))
}

#[cfg(not(windows))]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
extern "system" {
    fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
}

#[cfg(windows)]
fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let wide = |path: &Path| -> std::io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "update path contains a NUL character",
            ));
        }
        value.push(0);
        Ok(value)
    };
    let existing = wide(from)?;
    let replacement = wide(to)?;
    if unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A write that cannot succeed must fail now, and say why.
    ///
    /// The retry exists for Windows, where a file just written is routinely held
    /// open for a moment. rename(2) does not care about open handles, so on
    /// every other platform a failure is EXDEV, ENOSPC, EACCES or EROFS - none
    /// of which clear up in five seconds. Waiting anyway delays the state
    /// store's writes and then blames a cause that cannot be the cause.
    #[cfg(not(windows))]
    #[test]
    fn an_unrecoverable_replace_fails_at_once_and_names_the_real_cause() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::write(&source, b"payload").unwrap();
        // Renaming onto a path inside a file is EACCES or ENOTDIR - a failure
        // that no amount of waiting resolves.
        let blocked = source.join("inside-a-file").join("target");

        let started = std::time::Instant::now();
        let error = replace_file(&source, &blocked).unwrap_err();
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(1),
            "an unrecoverable rename waited {elapsed:?} before giving up"
        );
        let text = format!("{error:#}");
        assert!(
            !text.contains("held open by another process"),
            "a cause that cannot apply on this platform was reported: {text}"
        );
        assert!(
            text.contains("replace"),
            "the error must name the operation and path: {text}"
        );
    }

    /// A temporary left behind by a process that died mid-write must not block
    /// the next write.
    ///
    /// The state store used to name its temporary after the process id, and
    /// create it with create_new. So a temporary left by a dead process blocked
    /// every later write to that path from any process later given the same id.
    /// In a container ids start at 1 and are reused, which makes the next run
    /// the one that fails:
    ///
    ///   create /var/lib/invenqor-agent/last-heartbeat.tmp-1: File exists
    ///
    /// Found by the end-to-end run, which restarts the Agent in one container;
    /// on a normal host the ids are large enough that it takes a coincidence.
    #[test]
    fn a_temporary_left_by_a_dead_process_does_not_block_the_next_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("last-heartbeat");

        // Every shape a dead process could have left behind, including the one
        // the old naming would have produced.
        for leftover in [
            format!("last-heartbeat.tmp-{}", std::process::id()),
            "last-heartbeat.tmp-1".to_string(),
            "last-heartbeat.tmp-00000000-0000-4000-8000-000000000000".to_string(),
        ] {
            fs::write(directory.path().join(&leftover), b"half a write").unwrap();
        }

        atomic_write(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        atomic_write(&path, b"second").unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"second",
            "a repeated write must replace the file, not fail on its own leftovers"
        );
    }

    /// The temporary must not survive a successful write, or the state
    /// directory fills with them.
    #[test]
    fn a_successful_write_leaves_no_temporary_behind() {
        let directory = tempfile::tempdir().unwrap();
        atomic_write(&directory.path().join("status.json"), b"{}").unwrap();
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }
}
