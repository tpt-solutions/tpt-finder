// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 TPT Solutions

//! Opt-in local error log.
//!
//! Consistent with the local-first privacy stance: the log file lives in the
//! app data directory, is disabled by default, is never uploaded, and only
//! records error diagnostics (paths + messages) — no file contents.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct ErrorLog {
    path: PathBuf,
    enabled: AtomicBool,
    /// Serializes appends so concurrent workers don't interleave lines.
    write_lock: Mutex<()>,
}

impl ErrorLog {
    /// Log file at `<dir>/error.log`. Starts disabled; call `set_enabled`.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let mut path = dir.into();
        path.push("error.log");
        Self {
            path,
            enabled: AtomicBool::new(false),
            write_lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Append one timestamped line. Best-effort: never panics, never blocks
    /// long, and silently ignores write errors (logging must not take the
    /// app down).
    pub fn log(&self, ctx: &str, message: &str) {
        if !self.is_enabled() {
            return;
        }
        let _guard = self.write_lock.lock();
        let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        // Flatten newlines so each log() call is exactly one line.
        let flat = message.replace(['\n', '\r'], " ");
        let _ = writeln!(f, "{ts} [{ctx}] {flat}");
    }

    /// Best-effort size in bytes (0 when missing/unreadable).
    pub fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }

    /// Delete the log file (used when the user turns logging off).
    pub fn clear(&self) {
        let _guard = self.write_lock.lock();
        if let Ok(f) = File::open(&self.path) {
            drop(f);
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default_and_writes_only_when_enabled() {
        let dir = std::env::temp_dir().join(format!("tpt-errlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = ErrorLog::open(&dir);
        assert!(!log.is_enabled());

        log.log("test", "should not appear");
        assert_eq!(log.size_bytes(), 0);

        log.set_enabled(true);
        log.log("extract", "boom\nsecond line");
        log.log("ipc", "second");
        assert!(log.size_bytes() > 0);
        let contents = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(contents.lines().count(), 2);
        assert!(contents.contains("[extract] boom second line"));
        assert!(contents.contains("[ipc] second"));

        // Toggle off → silent again.
        log.set_enabled(false);
        log.log("test", "nope");
        assert_eq!(
            std::fs::read_to_string(log.path()).unwrap().lines().count(),
            2
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_removes_file() {
        let dir = std::env::temp_dir().join(format!("tpt-errlog2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = ErrorLog::open(&dir);
        log.set_enabled(true);
        log.log("test", "x");
        assert!(log.size_bytes() > 0);
        log.clear();
        assert_eq!(log.size_bytes(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
