use std::path::PathBuf;

use mlua::Table;

use crate::lua::{EngineContext, LuaEngine};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let mut path = std::env::temp_dir();
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        path.push(format!(
            "datamile_cache_test_{}_{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&path).expect("tempdir");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn engine_with_cache(cache: &TempDir) -> LuaEngine {
    let engine = LuaEngine::new().expect("engine");
    engine.lua.set_app_data(EngineContext::new(cache.0.clone()));
    engine
}

/// Verify cache hits avoid the Lua callback and content changes invalidate it.
#[test]
fn test_d_cache_file_hit_and_invalidation() {
    let tmp = TempDir::new();
    let cache = TempDir::new();
    let file = tmp.0.join("probe.bin");
    std::fs::write(&file, b"short").expect("file");
    let engine = engine_with_cache(&cache);

    let code = format!(
        r#"
        calls = 0
        local first = d.cache.file('probe', '{path}', function(path)
            calls = calls + 1
            return {{ size = #d.fs.read(path) }}
        end)
        local second = d.cache.file('probe', '{path}', function(path)
            calls = calls + 1
            return {{ size = #d.fs.read(path) }}
        end)
        return {{ calls = calls, size = first.size, second = second.size }}
    "#,
        path = file.to_string_lossy()
    );
    let res: Table = engine.lua.load(&code).eval().expect("cache");
    assert_eq!(res.get::<i64>("calls").unwrap(), 1);
    assert_eq!(res.get::<i64>("size").unwrap(), 5);
    assert_eq!(res.get::<i64>("second").unwrap(), 5);

    std::fs::write(&file, b"changed-size").expect("update");
    let calls: i64 = engine
        .lua
        .load(format!(
            r#"
            d.cache.file('probe', '{path}', function(path)
                calls = calls + 1
                return {{ size = #d.fs.read(path) }}
            end)
            return calls
        "#,
            path = file.to_string_lossy()
        ))
        .eval()
        .expect("cache after change");
    assert_eq!(calls, 2);
}

/// Verify namespaces isolate the same file and missing files return nil.
#[test]
fn test_d_cache_file_namespace_and_missing() {
    let tmp = TempDir::new();
    let cache = TempDir::new();
    let file = tmp.0.join("probe.bin");
    std::fs::write(&file, b"data").expect("file");
    let engine = engine_with_cache(&cache);

    let code = format!(
        r#"
        calls = 0
        local function probe(tag)
            return d.cache.file(tag, '{path}', function()
                calls = calls + 1
                return tag
            end)
        end
        return {{ a = probe('a'), b = probe('b'), calls = calls }}
    "#,
        path = file.to_string_lossy()
    );
    let res: Table = engine.lua.load(&code).eval().expect("namespaces");
    assert_eq!(res.get::<String>("a").unwrap(), "a");
    assert_eq!(res.get::<String>("b").unwrap(), "b");
    assert_eq!(res.get::<i64>("calls").unwrap(), 2);

    let missing = tmp.0.join("missing.bin");
    let code = format!(
        r#"return d.cache.file('probe', '{}', function() return {{}} end)"#,
        missing.to_string_lossy()
    );
    let value: mlua::Value = engine.lua.load(&code).eval().expect("missing");
    assert!(value.is_nil());

    let dirs = engine
        .lua
        .app_data_ref::<EngineContext>()
        .unwrap()
        .take_directories();
    assert!(dirs.contains(&tmp.0.canonicalize().unwrap()));
}

/// Verify empty namespaces and non-function callbacks are rejected.
#[test]
fn test_d_cache_file_validation() {
    let engine = LuaEngine::new().expect("engine");

    let empty = engine
        .lua
        .load("return d.cache.file('', '/tmp/x', function() return 1 end)")
        .eval::<mlua::Value>();
    assert!(empty.is_err());

    let bad_fn = engine
        .lua
        .load("return d.cache.file('ns', '/tmp/x', 'nope')")
        .eval::<mlua::Value>();
    assert!(bad_fn.is_err());
}
