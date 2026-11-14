use mlua::Table;

use crate::lua::LuaEngine;

/// Verify that `d.fn.present` returns the input value when the value is not empty.
#[test]
fn test_d_fn_present_valid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let result: String = engine
        .lua
        .load("return d.fn.present('hello')")
        .eval()
        .expect("Execution failed");
    assert_eq!(result, "hello");

    let num_res: f64 = engine
        .lua
        .load("return d.fn.present(123)")
        .eval()
        .expect("Execution failed");
    assert_eq!(num_res, 123.0);
}

/// Verify that `d.fn.present` returns an error for empty strings, whitespace strings, nil, and empty tables.
#[test]
fn test_d_fn_present_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let empty_str_res = engine.lua.load("return d.fn.present('')").eval::<String>();
    assert!(empty_str_res.is_err());

    let ws_res = engine
        .lua
        .load("return d.fn.present('   ')")
        .eval::<String>();
    assert!(ws_res.is_err());

    let nil_res = engine.lua.load("return d.fn.present(nil)").eval::<String>();
    assert!(nil_res.is_err());

    let empty_table_res = engine.lua.load("return d.fn.present({})").eval::<Table>();
    assert!(empty_table_res.is_err());
}

/// Verify that `d.fn.type_check` validates data types correctly, returns errors on mismatch, and allows missing values.
#[test]
fn test_d_fn_type_check() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let valid_str: String = engine
        .lua
        .load("return d.fn.type_check('sample', 'string')")
        .eval()
        .expect("Execution failed");
    assert_eq!(valid_str, "sample");

    let valid_num: f64 = engine
        .lua
        .load("return d.fn.type_check(42, 'number')")
        .eval()
        .expect("Execution failed");
    assert_eq!(valid_num, 42.0);

    let valid_bool: bool = engine
        .lua
        .load("return d.fn.type_check(true, 'boolean')")
        .eval()
        .expect("Execution failed");
    assert!(valid_bool);

    let valid_array: Table = engine
        .lua
        .load("return d.fn.type_check({1, 2}, 'array')")
        .eval()
        .expect("Execution failed");
    assert_eq!(valid_array.raw_len(), 2);

    let invalid_num = engine
        .lua
        .load("return d.fn.type_check(123, 'string')")
        .eval::<String>();
    assert!(invalid_num.is_err());

    let invalid_str = engine
        .lua
        .load("return d.fn.type_check('abc', 'number')")
        .eval::<f64>();
    assert!(invalid_str.is_err());

    let missing_nil: Option<String> = engine
        .lua
        .load("return d.fn.type_check(nil, 'string')")
        .eval()
        .expect("Execution failed");
    assert!(missing_nil.is_none());

    let missing_empty: Option<String> = engine
        .lua
        .load("return d.fn.type_check('', 'string')")
        .eval()
        .expect("Execution failed");
    assert!(missing_empty.is_none());
}

/// Verify that `d.fn.coalesce` returns the first value that is not empty.
#[test]
fn test_d_fn_coalesce() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let first_valid: String = engine
        .lua
        .load("return d.fn.coalesce('', nil, 'first', 'second')")
        .eval()
        .expect("Execution failed");
    assert_eq!(first_valid, "first");

    let table_valid: String = engine
        .lua
        .load("return d.fn.coalesce({}, '', 'from_table')")
        .eval()
        .expect("Execution failed");
    assert_eq!(table_valid, "from_table");

    let all_empty: Option<String> = engine
        .lua
        .load("return d.fn.coalesce('', nil, {})")
        .eval()
        .expect("Execution failed");
    assert!(all_empty.is_none());
}

/// Verify has matches scalars and scalar lists with exact/nocase options.
#[test]
fn test_d_fn_has() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        return {
            substring = d.fn.has('Aphex Twin', 'Twin'),
            case_sensitive = d.fn.has('Aphex Twin', 'twin'),
            nocase = d.fn.has('Aphex Twin', 'twin', { nocase = true }),
            exact_miss = d.fn.has('Aphex Twin', 'Aphex', { exact = true }),
            exact_hit = d.fn.has('Aphex Twin', 'Aphex Twin', { exact = true }),
            list_hit = d.fn.has({ 'rock', 'ambient' }, 'amb'),
            list_exact_miss = d.fn.has({ 'rock', 'ambient' }, 'amb', { exact = true }),
            number_hit = d.fn.has({ 1, 2, 3 }, 2),
            number_miss = d.fn.has(1, 2),
            nil_val = d.fn.has(nil, 'x'),
            nil_needle = d.fn.has('x', nil),
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("fn.has");

    assert!(res.get::<bool>("substring").unwrap());
    assert!(!res.get::<bool>("case_sensitive").unwrap());
    assert!(res.get::<bool>("nocase").unwrap());
    assert!(!res.get::<bool>("exact_miss").unwrap());
    assert!(res.get::<bool>("exact_hit").unwrap());
    assert!(res.get::<bool>("list_hit").unwrap());
    assert!(!res.get::<bool>("list_exact_miss").unwrap());
    assert!(res.get::<bool>("number_hit").unwrap());
    assert!(!res.get::<bool>("number_miss").unwrap());
    assert!(!res.get::<bool>("nil_val").unwrap());
    assert!(!res.get::<bool>("nil_needle").unwrap());
}
