//! The instance lock and the identity match rule a CLI applies before it
//! delegates anything.

use std::path::Path;

use super::{Identity, IdentityMismatch, InstanceError, InstanceLock};

fn lock_at(dir: &Path) -> std::path::PathBuf {
    dir.join("state/datamile/instance.lock")
}

/// The first holder owns the lock; a second holder is refused.
#[test]
fn second_acquire_is_refused() {
    let dir = tempfile::tempdir().expect("temp");
    let path = lock_at(dir.path());

    let first = InstanceLock::acquire_at(&path).expect("first lock");
    let err = InstanceLock::acquire_at(&path).expect_err("second lock");

    assert!(
        matches!(err, InstanceError::Busy { .. }),
        "unexpected: {err}"
    );
    drop(first);
}

/// The kernel releases the lock when the holder goes away, so the next taker
/// succeeds even though the file is still there.
#[test]
fn released_lock_is_acquirable() {
    let dir = tempfile::tempdir().expect("temp");
    let path = lock_at(dir.path());

    let first = InstanceLock::acquire_at(&path).expect("first lock");
    drop(first);

    assert!(path.exists(), "the lock file is not deleted");
    InstanceLock::acquire_at(&path).expect("lock after release");
}

/// `holder_at` reports the holder without keeping the lock, and reports none
/// when it is free.
#[test]
fn holder_reports_the_live_holder_only() {
    let dir = tempfile::tempdir().expect("temp");
    let path = lock_at(dir.path());

    assert_eq!(InstanceLock::holder_at(&path).expect("probe"), None);

    let lock = InstanceLock::acquire_at(&path).expect("lock");
    assert_eq!(
        InstanceLock::holder_at(&path).expect("probe"),
        Some(std::process::id())
    );

    drop(lock);
    assert_eq!(InstanceLock::holder_at(&path).expect("probe"), None);
}

fn identity(config: &str, version: &str, reload_error: Option<&str>) -> Identity {
    Identity {
        config_path: config.to_string(),
        version: version.to_string(),
        reload_error: reload_error.map(ToString::to_string),
    }
}

/// The match rule: same canonical config, same version, no reload error.
#[test]
fn identity_matches_only_the_same_config_and_version() {
    let dir = tempfile::tempdir().expect("temp");
    let config = dir.path().join("init.lua");
    std::fs::write(&config, "").expect("config");
    let canonical = config.canonicalize().expect("canonical");

    let ours = identity(&canonical.to_string_lossy(), "1.0", None);
    assert!(ours.check(&config, "1.0").is_ok());

    let other = identity("/elsewhere/init.lua", "1.0", None);
    assert!(matches!(
        other.check(&config, "1.0"),
        Err(IdentityMismatch::Config(_))
    ));

    let older = identity(&canonical.to_string_lossy(), "0.9", None);
    assert!(matches!(
        older.check(&config, "1.0"),
        Err(IdentityMismatch::Version { .. })
    ));

    let broken = identity(&canonical.to_string_lossy(), "1.0", Some("boom"));
    assert!(matches!(
        broken.check(&config, "1.0"),
        Err(IdentityMismatch::Reload(_))
    ));
}
