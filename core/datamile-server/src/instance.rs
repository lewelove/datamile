use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use thiserror::Error;

#[cfg(test)]
mod tests;

/// The daemon's version, compared by a CLI before it delegates anything.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const LOCK_DIR_MODE: u32 = 0o700;

#[derive(Debug, Error)]
pub enum InstanceError {
    #[error("another datamile holds the instance lock ({path}): {holder}")]
    Busy { path: PathBuf, holder: String },

    #[error("cannot use the instance lock {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The lock path comes from `$HOME` only: `XDG_STATE_HOME` is ignored on
/// purpose, so cron, systemd, SSH and a desktop session all see one lock.
#[must_use]
pub fn lock_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local/state/datamile/instance.lock"))
}

/// Exclusive, kernel-held lock for the whole life of the holder.
#[derive(Debug)]
pub struct InstanceLock {
    file: File,
}

impl InstanceLock {
    /// Takes the lock or reports who holds it. Liveness is the lock itself; the
    /// pid in the file is only there for the error message.
    pub fn acquire() -> Result<Self, InstanceError> {
        let path = lock_path().ok_or_else(|| InstanceError::Io {
            path: PathBuf::from("~/.local/state/datamile/instance.lock"),
            source: std::io::Error::other("$HOME is not set"),
        })?;

        Self::acquire_at(&path)
    }

    /// Same lock, at an explicit path, so tests never touch `$HOME`.
    pub fn acquire_at(path: &Path) -> Result<Self, InstanceError> {
        create_dir(path)?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|source| InstanceError::Io {
                path: path.to_path_buf(),
                source,
            })?;

        // SAFETY: `flock` only touches the fd we own for the life of `file`.
        let locked =
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            let source = std::io::Error::last_os_error();
            if source.kind() != std::io::ErrorKind::WouldBlock {
                return Err(InstanceError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }

            return Err(InstanceError::Busy {
                holder: holder_description(path),
                path: path.to_path_buf(),
            });
        }

        let mut lock = Self { file };
        lock.write_pid();

        Ok(lock)
    }

    /// Whether a daemon is live, without keeping the lock.
    pub fn holder() -> Result<Option<u32>, InstanceError> {
        let Some(path) = lock_path() else {
            return Ok(None);
        };
        Self::holder_at(&path)
    }

    pub fn holder_at(path: &Path) -> Result<Option<u32>, InstanceError> {
        match Self::acquire_at(path) {
            Ok(_lock) => Ok(None),
            Err(InstanceError::Busy { .. }) => Ok(read_pid(path)),
            Err(e) => Err(e),
        }
    }

    fn write_pid(&mut self) {
        let pid = std::process::id();
        let _ = self.file.set_len(0);
        let _ = self.file.write_all(pid.to_string().as_bytes());
        let _ = self.file.flush();
    }
}

fn create_dir(lock: &Path) -> Result<(), InstanceError> {
    let Some(dir) = lock.parent() else {
        return Ok(());
    };

    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(LOCK_DIR_MODE);
    builder.create(dir).map_err(|source| InstanceError::Io {
        path: dir.to_path_buf(),
        source,
    })
}

fn read_pid(path: &Path) -> Option<u32> {
    let mut content = String::new();
    File::open(path).ok()?.read_to_string(&mut content).ok()?;

    content.trim().parse().ok()
}

fn holder_description(path: &Path) -> String {
    read_pid(path).map_or_else(
        || format!("an unknown process holding {}", path.display()),
        |pid| format!("pid {pid}"),
    )
}

/// What a daemon says about itself at `GET /api/identity`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub config_path: String,
    pub version: String,
    pub reload_error: Option<String>,
}

/// Why a daemon may not serve this CLI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityMismatch {
    #[error("the daemon serves a different config ({0}); refusing to delegate")]
    Config(String),

    #[error(
        "the daemon runs version {running}, this CLI is {mine}; refusing to delegate"
    )]
    Version { running: String, mine: String },

    #[error("the daemon failed its last config reload: {0}; refusing to delegate")]
    Reload(String),
}

impl Identity {
    /// The match rule: canonical config path equal, version exactly equal, and
    /// no reload error.
    pub fn check(
        &self,
        config_path: &Path,
        version: &str,
    ) -> Result<(), IdentityMismatch> {
        let canonical = config_path
            .canonicalize()
            .unwrap_or_else(|_| config_path.to_path_buf());

        if canonical != Path::new(&self.config_path) {
            return Err(IdentityMismatch::Config(self.config_path.clone()));
        }

        if self.version != version {
            return Err(IdentityMismatch::Version {
                running: self.version.clone(),
                mine: version.to_string(),
            });
        }

        if let Some(error) = &self.reload_error {
            return Err(IdentityMismatch::Reload(error.clone()));
        }

        Ok(())
    }
}
