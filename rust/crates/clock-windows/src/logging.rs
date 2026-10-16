//! Bounded local transition log (R11).
//!
//! Plain text lines, one per state transition (never per repaint), written to the
//! app's own per-user folder. At most [`MAX_FILES`] files of [`MAX_FILE_BYTES`] each
//! are kept, so the log can never grow past about 3 MiB. Nothing is uploaded and
//! nothing is sent over a network.
//!
//! Logging is strictly best effort: every failure (no folder, disk full, a file held
//! open by a virus scanner) is swallowed. The clock never depends on the log, and a
//! write that fails is simply dropped. Callers pass text that is already free of
//! personal data; [`log`] additionally runs everything through [`support::redact`].
use crate::support;
use chrono::DateTime;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

/// Largest size one log file may reach.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Number of log files kept, the current one included.
pub const MAX_FILES: usize = 3;
/// A single line is cut here so that one message can never exceed a file.
const MAX_LINE_BYTES: usize = 1024;

const BASE: &str = "elevenclock-rs";

/// A size-bounded rotating text log inside one folder.
///
/// Files are `elevenclock-rs.log` (current), `elevenclock-rs.1.log` and
/// `elevenclock-rs.2.log` (older). The folder is created on first write.
pub struct RotatingLog {
    directory: PathBuf,
    max_bytes: u64,
    max_files: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingLog {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self::with_limits(directory, MAX_FILE_BYTES, MAX_FILES)
    }

    /// Smaller limits make rotation testable without writing megabytes.
    pub fn with_limits(directory: impl Into<PathBuf>, max_bytes: u64, max_files: usize) -> Self {
        Self {
            directory: directory.into(),
            max_bytes: max_bytes.max(1),
            max_files: max_files.max(1),
            file: None,
            size: 0,
        }
    }

    fn path(&self, index: usize) -> PathBuf {
        file_path(&self.directory, index)
    }

    /// Append one line (a newline is added). On error the line is dropped and the
    /// next call starts from a fresh look at the disk.
    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        let mut text = truncate(line, MAX_LINE_BYTES.min(self.max_bytes as usize - 1)).to_owned();
        text.push('\n');
        let result = self.append(text.as_bytes());
        if result.is_err() {
            // Reopen (and re-measure) next time rather than trust a broken handle.
            self.file = None;
        }
        result
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.file.is_none() {
            fs::create_dir_all(&self.directory)?;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.path(0))?;
            self.size = file.metadata()?.len();
            self.file = Some(file);
        }
        if self.size > 0 && self.size + bytes.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("log closed"))?;
        file.write_all(bytes)?;
        self.size += bytes.len() as u64;
        Ok(())
    }

    /// Shift every file one place older, dropping the oldest, and start afresh.
    fn rotate(&mut self) -> io::Result<()> {
        // Release the handle first: Windows will not rename an open file we hold.
        self.file = None;
        let oldest = self.max_files - 1;
        if oldest == 0 {
            // Only one file is allowed: it is simply emptied.
            fs::remove_file(self.path(0))?;
        } else {
            let _ = fs::remove_file(self.path(oldest));
            for index in (0..oldest).rev() {
                match fs::rename(self.path(index), self.path(index + 1)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(0))?;
        self.size = 0;
        self.file = Some(file);
        Ok(())
    }
}

fn file_path(directory: &Path, index: usize) -> PathBuf {
    if index == 0 {
        directory.join(format!("{BASE}.log"))
    } else {
        directory.join(format!("{BASE}.{index}.log"))
    }
}

/// Cut at a character boundary so the result is still valid UTF-8.
fn truncate(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

static LOG: Mutex<Option<RotatingLog>> = Mutex::new(None);

/// Start logging into the app's folder. Safe to call when the folder cannot be
/// resolved: logging then stays off and nothing else is affected.
#[cfg(windows)]
pub fn init() {
    if let Ok(directory) = crate::preferences_store::app_data_dir() {
        init_in(directory.join("logs"));
    }
}

/// Start logging into `directory` (used by `init` and by tests).
pub fn init_in(directory: impl Into<PathBuf>) {
    if let Ok(mut slot) = LOG.lock() {
        *slot = Some(RotatingLog::new(directory));
    }
}

/// Whether anything would be written; lets callers skip building messages.
pub fn enabled() -> bool {
    LOG.lock().is_ok_and(|slot| slot.is_some())
}

/// Record one transition. A no-op until logging is started, and never fails.
pub fn log(message: &str) {
    let flat = message.replace(['\r', '\n'], " ");
    let line = format!("{} {}", timestamp(), support::redact(&flat));
    if let Ok(mut slot) = LOG.lock()
        && let Some(log) = slot.as_mut()
    {
        let _ = log.write_line(&line);
    }
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .map_or_else(
            || "unknown-time".to_owned(),
            |time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn rotation_keeps_three_files_none_larger_than_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = RotatingLog::with_limits(directory.path(), 100, 3);
        for number in 0..200 {
            log.write_line(&format!("transition number {number:04}"))
                .unwrap();
        }
        assert_eq!(
            names(directory.path()),
            [
                "elevenclock-rs.1.log",
                "elevenclock-rs.2.log",
                "elevenclock-rs.log"
            ]
        );
        for name in names(directory.path()) {
            let size = fs::metadata(directory.path().join(name)).unwrap().len();
            assert!(size <= 100, "{size}");
        }
        // The newest line is in the current file, and older lines were dropped.
        let current = fs::read_to_string(directory.path().join("elevenclock-rs.log")).unwrap();
        assert!(current.contains("transition number 0199"));
        let oldest = fs::read_to_string(directory.path().join("elevenclock-rs.2.log")).unwrap();
        assert!(!oldest.contains("transition number 0000"));
    }

    #[test]
    fn production_limits_are_one_mebibyte_and_three_files() {
        assert_eq!(MAX_FILE_BYTES, 1_048_576);
        assert_eq!(MAX_FILES, 3);
        let directory = tempfile::tempdir().unwrap();
        let mut log = RotatingLog::new(directory.path());
        let line = "x".repeat(1000);
        // About 4 MiB of lines: several rotations, still three files at most.
        for _ in 0..4200 {
            log.write_line(&line).unwrap();
        }
        let names = names(directory.path());
        assert_eq!(names.len(), 3);
        for name in names {
            assert!(fs::metadata(directory.path().join(name)).unwrap().len() <= MAX_FILE_BYTES);
        }
    }

    #[test]
    fn a_long_line_is_cut_and_cannot_exceed_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = RotatingLog::with_limits(directory.path(), 64, 2);
        log.write_line(&"é".repeat(500)).unwrap();
        let text = fs::read_to_string(directory.path().join("elevenclock-rs.log")).unwrap();
        assert!(text.len() <= 64 && text.ends_with('\n'));
    }

    #[test]
    fn an_existing_oversized_file_is_rotated_before_appending() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("elevenclock-rs.log"), vec![b'a'; 500]).unwrap();
        let mut log = RotatingLog::with_limits(directory.path(), 100, 3);
        log.write_line("fresh").unwrap();
        assert_eq!(
            fs::read_to_string(directory.path().join("elevenclock-rs.log")).unwrap(),
            "fresh\n"
        );
        assert_eq!(
            fs::metadata(directory.path().join("elevenclock-rs.1.log"))
                .unwrap()
                .len(),
            500
        );
    }

    #[test]
    fn disk_errors_are_reported_to_the_caller_never_panic_and_recover() {
        let directory = tempfile::tempdir().unwrap();
        // A *file* where the log folder should be: the folder cannot be created.
        let blocker = directory.path().join("logs");
        fs::write(&blocker, b"not a folder").unwrap();
        let mut log = RotatingLog::new(&blocker);
        assert!(log.write_line("lost").is_err());
        assert!(log.write_line("lost again").is_err());
        // The obstruction goes away: the very next write works.
        fs::remove_file(&blocker).unwrap();
        log.write_line("back").unwrap();
        assert_eq!(
            fs::read_to_string(blocker.join("elevenclock-rs.log")).unwrap(),
            "back\n"
        );
    }

    #[test]
    fn a_file_that_cannot_be_rotated_fails_the_write_not_the_process() {
        let directory = tempfile::tempdir().unwrap();
        let mut log = RotatingLog::with_limits(directory.path(), 20, 2);
        log.write_line("0123456789").unwrap();
        // Occupy the rotation target with a non-empty folder so the rename fails.
        let target = directory.path().join("elevenclock-rs.1.log");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"x").unwrap();
        assert!(log.write_line("abcdefghij").is_err());
    }

    #[test]
    fn global_log_is_a_no_op_until_started_and_redacts() {
        log("before init is ignored without error");
        let directory = tempfile::tempdir().unwrap();
        init_in(directory.path());
        log(r"opened C:\Users\someone\AppData\Local\thing failed");
        let text = fs::read_to_string(directory.path().join("elevenclock-rs.log")).unwrap();
        assert!(text.contains("<path>"), "{text}");
        assert!(!text.contains("someone"), "{text}");
        assert!(text.starts_with("20"), "{text}");
        // Leave logging off for other tests in this process.
        *LOG.lock().unwrap() = None;
    }
}
