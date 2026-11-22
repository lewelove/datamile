use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::lua::resolve_config;

/// Creates `<base>/<name>/init.lua` and returns the directory holding it.
fn config_dir(base: &Path, name: &str) -> PathBuf {
    let dir = base.join(name);
    fs::create_dir_all(&dir).expect("create config dir");
    fs::write(dir.join("init.lua"), "").expect("write init.lua");
    dir
}

#[test]
fn the_flag_wins_over_env_and_home() {
    let tmp = TempDir::new().expect("temp dir");
    let flag = config_dir(tmp.path(), "flag");
    let env = config_dir(tmp.path(), "env");
    let home = config_dir(tmp.path(), "home");

    let found = resolve_config(
        Some(&flag),
        Some(&env),
        Some(&home.join("init.lua")),
        tmp.path(),
    )
    .expect("resolution must succeed");

    assert_eq!(found, Some(flag.join("init.lua")));
}

#[test]
fn the_env_var_wins_over_home() {
    let tmp = TempDir::new().expect("temp dir");
    let env = config_dir(tmp.path(), "env");
    let home = config_dir(tmp.path(), "home");

    let found =
        resolve_config(None, Some(&env), Some(&home.join("init.lua")), tmp.path())
            .expect("resolution must succeed");

    assert_eq!(found, Some(env.join("init.lua")));
}

/// The working directory still holds an init.lua, so this also proves the home
/// config is consulted before the walk-up.
#[test]
fn home_wins_over_the_working_directory() {
    let tmp = TempDir::new().expect("temp dir");
    let home = config_dir(tmp.path(), "home");
    config_dir(tmp.path(), "cwd");

    let found = resolve_config(None, None, Some(&home.join("init.lua")), tmp.path())
        .expect("resolution must succeed");

    assert_eq!(found, Some(home.join("init.lua")));
}

#[test]
fn the_working_directory_is_the_last_resort() {
    let tmp = TempDir::new().expect("temp dir");
    let nested = tmp.path().join("project").join("config");
    fs::create_dir_all(&nested).expect("create nested dir");
    fs::write(nested.join("init.lua"), "").expect("write init.lua");

    let found =
        resolve_config(None, None, None, &tmp.path().join("project").join("deep"))
            .expect("resolution must succeed");

    assert_eq!(
        found,
        Some(tmp.path().join("project").join("config").join("init.lua"))
    );
}

#[test]
fn a_config_file_is_rejected() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = config_dir(tmp.path(), "cfg");

    let err = resolve_config(Some(&dir.join("init.lua")), None, None, tmp.path())
        .expect_err("a file is not a config directory");

    assert!(
        err.to_string().contains("got a file"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_missing_directory_is_rejected() {
    let tmp = TempDir::new().expect("temp dir");

    let err = resolve_config(Some(&tmp.path().join("gone")), None, None, tmp.path())
        .expect_err("a missing directory cannot be the config root");

    assert!(
        err.to_string().contains("does not exist"),
        "unexpected error: {err}"
    );
}

#[test]
fn a_directory_without_init_lua_is_rejected() {
    let tmp = TempDir::new().expect("temp dir");
    let empty = tmp.path().join("empty");
    fs::create_dir_all(&empty).expect("create empty dir");

    let err = resolve_config(Some(&empty), None, None, tmp.path())
        .expect_err("a directory without init.lua is unusable");

    assert!(
        err.to_string().contains("no init.lua"),
        "unexpected error: {err}"
    );
}

#[test]
fn nothing_anywhere_resolves_to_none() {
    let tmp = TempDir::new().expect("temp dir");

    let found =
        resolve_config(None, None, None, &tmp.path().join("deep")).expect("no error");

    assert_eq!(found, None);
}
