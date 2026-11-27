//! Contract of the per-entity reader memo and of reader injection.

use std::fs;

use mlua::{Table, Value};

use crate::LuaRuntime;

/// A reader is computed once per entity: editing the file mid-entity keeps
/// returning the first value until the memo is cleared.
#[test]
fn read_toml_is_memoized_until_cleared() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("meta.toml");
    fs::write(&path, "album = \"first\"\n").expect("write first");

    let engine = LuaRuntime::new().expect("runtime");
    let code = format!("return d.fs.read_toml('{}').album", path.display());

    let first: String = engine.lua.load(&code).eval().expect("first read");
    assert_eq!(first, "first");

    fs::write(&path, "album = \"second\"\n").expect("write second");

    let cached: String = engine.lua.load(&code).eval().expect("cached read");
    assert_eq!(cached, "first");

    engine.clear_entity_memo();

    let refreshed: String = engine.lua.load(&code).eval().expect("fresh read");
    assert_eq!(refreshed, "second");
}

/// Reading a file records it as a dependency, memo hit or not.
#[test]
fn reads_record_dependencies() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("meta.toml");
    fs::write(&path, "a = 1\n").expect("write");

    let engine = LuaRuntime::new().expect("runtime");
    let code = format!("return d.fs.read_toml('{}')", path.display());
    engine.lua.load(&code).exec().expect("read");

    let deps = engine.take_dependencies();
    assert!(deps.contains(&path.canonicalize().expect("canonical")));
}

/// A memo hit never bypasses the compile guard: the path is refused even
/// though its converted value is already in hand.
#[test]
fn memoized_lock_read_is_refused_while_compiling() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("album.lock.json");
    fs::write(&path, r#"{"album":{}}"#).expect("write lock");

    let engine = LuaRuntime::new().expect("runtime");
    let code = format!("return d.fs.read_json('{}')", path.display());
    engine
        .lua
        .load(&code)
        .exec()
        .expect("read before the compile");

    let guard = engine.compile_guard().expect("guard");
    let err = engine
        .lua
        .load(&code)
        .eval::<Value>()
        .expect_err("a lock must be refused");
    assert!(format!("{err}").contains("a lock is never an input"));

    drop(guard);
}

/// Injected readers are visible through both aliases and share the memo.
#[test]
fn registered_fs_function_is_visible_on_both_aliases() {
    let engine = LuaRuntime::new().expect("runtime");
    engine
        .register_fs_function("read_probe", |lua, _path| {
            let table = lua.create_table()?;
            table.set("ok", true)?;
            Ok(Value::Table(table))
        })
        .expect("register");

    let via_d: Table = engine
        .lua
        .load("return d.fs.read_probe('/tmp/anything')")
        .eval()
        .expect("d.fs");
    assert!(via_d.get::<bool>("ok").expect("ok"));

    let via_datamile: Table = engine
        .lua
        .load("return datamile.fs.read_probe('/tmp/anything')")
        .eval()
        .expect("datamile.fs");
    assert!(via_datamile.get::<bool>("ok").expect("ok"));
}
