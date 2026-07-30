//! A log file for hosts where the journal is not where the logs go.
//!
//! A Windows service has no console, and its standard error is a handle that
//! leads nowhere. Everything the Agent logged while running as a service was
//! therefore discarded, which is how a service that never completed a single
//! collection could look healthy: `Get-Service` said Running, `--diagnose` said
//! the Server was reachable, and the one thing that would have explained it - the
//! Agent's own log - did not exist anywhere.
//!
//! So the Agent writes its log beside its state. The file is capped and rotated
//! once, because an inventory agent must not be the reason a disk fills.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Rotate at 8 MiB and keep one previous file, so at most 16 MiB is used.
const MAXIMUM_BYTES: u64 = 8 * 1024 * 1024;

pub struct LogFile {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl LogFile {
    /// Opens the log beside the state directory. Returns None when the directory
    /// cannot be written, in which case logging simply stays on stderr rather
    /// than the Agent refusing to run.
    pub fn open(state_dir: &Path) -> Option<Self> {
        let path = state_dir.join("agent.log");
        if fs::create_dir_all(state_dir).is_err() {
            return None;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(Self {
            path,
            file: Mutex::new(Some(file)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn rotate_if_large(&self, file: &mut Option<File>) {
        let too_large = file
            .as_ref()
            .and_then(|handle| handle.metadata().ok())
            .is_some_and(|metadata| metadata.len() >= MAXIMUM_BYTES);
        if !too_large {
            return;
        }
        // Drop the handle before renaming: Windows will not rename a file that is
        // still open for writing.
        *file = None;
        let previous = self.path.with_extension("log.1");
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(&self.path, &previous);
        *file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
    }
}

impl Write for &LogFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.file.lock() else {
            // A poisoned lock must not take the process down over a log line.
            return Ok(bytes.len());
        };
        self.rotate_if_large(&mut guard);
        match guard.as_mut() {
            Some(file) => file.write(bytes),
            None => Ok(bytes.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let Ok(mut guard) = self.file.lock() else {
            return Ok(());
        };
        match guard.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_rotates_once_at_the_cap() {
        let root = std::env::temp_dir().join(format!("iq-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let log = LogFile::open(&root).expect("open log");
        let mut writer = &log;
        writer.write_all(b"first line\n").unwrap();
        writer.flush().unwrap();
        assert!(log.path().exists());
        assert!(fs::read_to_string(log.path())
            .unwrap()
            .contains("first line"));

        // Fill past the cap and confirm the file is rotated rather than growing
        // without bound: a full disk caused by the agent's own log would be a
        // worse failure than the one the log exists to explain.
        let block = vec![b'x'; 64 * 1024];
        for _ in 0..(MAXIMUM_BYTES / block.len() as u64 + 2) {
            writer.write_all(&block).unwrap();
        }
        writer.flush().unwrap();
        writer.write_all(b"after rotation\n").unwrap();
        writer.flush().unwrap();
        let previous = log.path().with_extension("log.1");
        assert!(previous.exists(), "the previous log must be kept");
        let current = fs::read_to_string(log.path()).unwrap();
        assert!(current.contains("after rotation"));
        assert!(
            current.len() < MAXIMUM_BYTES as usize,
            "the current log must start over after rotation"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unwritable_directory_leaves_logging_on_stderr() {
        // A path that cannot be a directory: the Agent must still start.
        let file = std::env::temp_dir().join(format!("iq-not-a-dir-{}", std::process::id()));
        fs::write(&file, b"x").unwrap();
        assert!(LogFile::open(&file.join("state")).is_none());
        let _ = fs::remove_file(&file);
    }
}
