//! Advisory pidfile. On acquire:
//!   1. Open (create if missing) `<path>` read-write.
//!   2. `fs2::try_lock_exclusive` — if another process holds the lock, read
//!      its contents and return [`DaemonError::AlreadyRunning`].
//!   3. Rewind, truncate, write current PID + `\n`.
//!
//! On drop / explicit `release()`: unlock + unlink (best effort).
//!
//! The advisory lock is flock(2)-based via `fs2`; it's unaffected by file
//! copies and released if the holder dies without cleanup (kernel releases
//! the lock on fd close). That covers the SIGKILL case: a stale PID file on
//! disk has no lock, so the next daemon acquires it cleanly.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::{DaemonError, DaemonResult};

/// Owning handle on the pidfile. Lock released when dropped.
pub struct PidFile {
    path: PathBuf,
    file: Option<File>,
}

impl std::fmt::Debug for PidFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PidFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PidFile {
    /// Try to acquire the lock and write the current PID.
    ///
    /// # Errors
    /// - [`DaemonError::AlreadyRunning`] if another process holds the lock.
    /// - [`DaemonError::Pidfile`] on any underlying IO failure.
    pub fn acquire(path: impl AsRef<Path>) -> DaemonResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DaemonError::Pidfile {
                path: path.clone(),
                source: e,
            })?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| DaemonError::Pidfile {
                path: path.clone(),
                source: e,
            })?;

        if file.try_lock_exclusive().is_err() {
            let mut s = String::new();
            let _ = file.read_to_string(&mut s);
            let pid: u32 = s.trim().parse().unwrap_or(0);
            return Err(DaemonError::AlreadyRunning { pid, path });
        }

        file.set_len(0).map_err(|e| DaemonError::Pidfile {
            path: path.clone(),
            source: e,
        })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| DaemonError::Pidfile {
                path: path.clone(),
                source: e,
            })?;
        writeln!(file, "{}", std::process::id()).map_err(|e| DaemonError::Pidfile {
            path: path.clone(),
            source: e,
        })?;
        file.flush().map_err(|e| DaemonError::Pidfile {
            path: path.clone(),
            source: e,
        })?;

        Ok(Self {
            path,
            file: Some(file),
        })
    }

    /// The path this lock is anchored to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// PID recorded inside the file (always this process on a live handle).
    #[must_use]
    pub fn pid(&self) -> u32 {
        std::process::id()
    }

    /// Explicitly release (unlock + unlink). Drop does the same, but in an
    /// ordered shutdown we want errors logged rather than swallowed.
    ///
    /// # Errors
    /// IO errors during unlink.
    pub fn release(mut self) -> DaemonResult<()> {
        if let Some(file) = self.file.take() {
            let _ = fs2::FileExt::unlock(&file);
            drop(file);
        }
        std::fs::remove_file(&self.path).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(DaemonError::Pidfile {
                    path: self.path.clone(),
                    source: e,
                })
            }
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = fs2::FileExt::unlock(&file);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn acquire_creates_pidfile_with_current_pid() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let lock = PidFile::acquire(&p).unwrap();
        assert!(p.exists());
        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s.trim().parse::<u32>().unwrap(), std::process::id());
        drop(lock);
    }

    #[test]
    fn second_acquire_in_same_process_returns_already_running() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let _first = PidFile::acquire(&p).unwrap();
        let err = PidFile::acquire(&p).unwrap_err();
        match err {
            DaemonError::AlreadyRunning { pid, .. } => assert_eq!(pid, std::process::id()),
            other => panic!("wrong err: {other:?}"),
        }
    }

    #[test]
    fn release_unlinks_pidfile() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        let lock = PidFile::acquire(&p).unwrap();
        lock.release().unwrap();
        assert!(!p.exists());
    }

    #[test]
    fn drop_unlinks_pidfile() {
        let d = tempdir().unwrap();
        let p = d.path().join("daemon.pid");
        {
            let _lock = PidFile::acquire(&p).unwrap();
            assert!(p.exists());
        }
        assert!(!p.exists());
    }

    #[test]
    fn parent_dir_is_created_on_acquire() {
        let d = tempdir().unwrap();
        let p = d.path().join("nested").join("dir").join("daemon.pid");
        let _lock = PidFile::acquire(&p).unwrap();
        assert!(p.exists());
    }
}
