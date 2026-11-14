use mlua::Table;

use crate::lua::LuaEngine;

/// Verify that `d.tbl.extend` merges tables shallowly and respects behavior modes.
#[test]
fn test_d_tbl_extend() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let force_res: Table = engine
        .lua
        .load("return d.tbl.extend('force', { a = 1, b = 2 }, { b = 3, c = 4 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(force_res.get::<i64>("a").unwrap(), 1);
    assert_eq!(force_res.get::<i64>("b").unwrap(), 3);
    assert_eq!(force_res.get::<i64>("c").unwrap(), 4);

    let keep_res: Table = engine
        .lua
        .load("return d.tbl.extend('keep', { a = 1, b = 2 }, { b = 3, c = 4 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(keep_res.get::<i64>("b").unwrap(), 2);
    assert_eq!(keep_res.get::<i64>("c").unwrap(), 4);

    let multi_res: Table = engine
        .lua
        .load("return d.tbl.extend('force', { a = 1 }, nil, { b = 2 }, { c = 3 }, { a = 9 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(multi_res.get::<i64>("a").unwrap(), 9);
    assert_eq!(multi_res.get::<i64>("b").unwrap(), 2);
    assert_eq!(multi_res.get::<i64>("c").unwrap(), 3);

    let empty_call: Table = engine
        .lua
        .load("return d.tbl.extend('force')")
        .eval()
        .expect("Execution failed");
    assert!(empty_call.raw_len() == 0);

    let single_table: Table = engine
        .lua
        .load("local src = { a = 10 }; local out = d.tbl.extend('force', src); out.a = 20; return { src = src.a, out = out.a }")
        .eval()
        .expect("Execution failed");
    assert_eq!(single_table.get::<i64>("src").unwrap(), 10);
    assert_eq!(single_table.get::<i64>("out").unwrap(), 20);

    let fn_res: Table = engine
        .lua
        .load("return d.tbl.extend(function(k, v1, v2) return (v1 or 0) + v2 end, { a = 10 }, { a = 5 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(fn_res.get::<i64>("a").unwrap(), 15);

    let err_conflict = engine
        .lua
        .load("return d.tbl.extend('error', { a = 1 }, { a = 2 })")
        .eval::<Table>();
    assert!(err_conflict.is_err());

    let invalid_behavior = engine
        .lua
        .load("return d.tbl.extend('invalid_mode', { a = 1 })")
        .eval::<Table>();
    assert!(invalid_behavior.is_err());

    let non_table_arg = engine
        .lua
        .load("return d.tbl.extend('force', { a = 1 }, 123)")
        .eval::<Table>();
    assert!(non_table_arg.is_err());

    let non_table_first = engine
        .lua
        .load("return d.tbl.extend('force', 'bad_string')")
        .eval::<Table>();
    assert!(non_table_first.is_err());
}

/// Verify that `d.tbl.deep_extend` merges tables recursively and treats lists as values.
#[test]
fn test_d_tbl_deep_extend() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local t1 = { nested = { a = 1, list = { 1, 2 }, deeper = { x = 10 } } }
        local t2 = { nested = { b = 2, list = { 3 }, deeper = { y = 20 } } }
        local t3 = { nested = { deeper = { z = 30 } } }
        return d.tbl.deep_extend('force', t1, t2, t3)
    "#;
    let res: Table = engine.lua.load(code).eval().expect("Execution failed");
    let nested: Table = res.get("nested").unwrap();
    assert_eq!(nested.get::<i64>("a").unwrap(), 1);
    assert_eq!(nested.get::<i64>("b").unwrap(), 2);
    let deeper: Table = nested.get("deeper").unwrap();
    assert_eq!(deeper.get::<i64>("x").unwrap(), 10);
    assert_eq!(deeper.get::<i64>("y").unwrap(), 20);
    assert_eq!(deeper.get::<i64>("z").unwrap(), 30);
    let list: Table = nested.get("list").unwrap();
    assert_eq!(list.raw_len(), 1);
    assert_eq!(list.get::<i64>(1).unwrap(), 3);

    let empty_merge: Table = engine
        .lua
        .load("return d.tbl.deep_extend('force', { a = {} }, { a = { b = 1 } })")
        .eval()
        .expect("Execution failed");
    let a_tbl: Table = empty_merge.get("a").unwrap();
    assert_eq!(a_tbl.get::<i64>("b").unwrap(), 1);

    let deep_err = engine
        .lua
        .load("return d.tbl.deep_extend('error', { nested = { a = 1 } }, { nested = { a = 2 } })")
        .eval::<Table>();
    assert!(deep_err.is_err());
}

/// Verify that `d.tbl.contains` checks values and predicate functions.
#[test]
fn test_d_tbl_contains() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let val_found: bool = engine
        .lua
        .load("return d.tbl.contains({ 'rock', 'pop', 'ambient' }, 'pop')")
        .eval()
        .expect("Execution failed");
    assert!(val_found);

    let val_not_found: bool = engine
        .lua
        .load("return d.tbl.contains({ 'rock', 'pop' }, 'classical')")
        .eval()
        .expect("Execution failed");
    assert!(!val_not_found);

    let map_val_found: bool = engine
        .lua
        .load("return d.tbl.contains({ genre = 'ambient', year = 2024 }, 'ambient')")
        .eval()
        .expect("Execution failed");
    assert!(map_val_found);

    let bool_found: bool = engine
        .lua
        .load("return d.tbl.contains({ true, false }, false)")
        .eval()
        .expect("Execution failed");
    assert!(bool_found);

    let table_ref_found: bool = engine
        .lua
        .load(
            "local item = { id = 1 }; return d.tbl.contains({ item, { id = 2 } }, item)",
        )
        .eval()
        .expect("Execution failed");
    assert!(table_ref_found);

    let pred_found: bool = engine
        .lua
        .load("return d.tbl.contains({ 10, 20, 30 }, function(v) return v > 25 end, { predicate = true })")
        .eval()
        .expect("Execution failed");
    assert!(pred_found);

    let pred_not_found: bool = engine
        .lua
        .load("return d.tbl.contains({ 10, 20 }, function(v) return v > 25 end, { predicate = true })")
        .eval()
        .expect("Execution failed");
    assert!(!pred_not_found);

    let pred_type_err = engine
        .lua
        .load("return d.tbl.contains({ 1, 2 }, 'not_fn', { predicate = true })")
        .eval::<bool>();
    assert!(pred_type_err.is_err());

    let non_table_contains: bool = engine
        .lua
        .load("return d.tbl.contains('not_table', 'a')")
        .eval()
        .expect("Execution failed");
    assert!(!non_table_contains);
}

/// Verify that `d.tbl.filter` and `d.tbl.map` transform and filter tables correctly.
#[test]
fn test_d_tbl_filter_and_map() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let filtered: Table = engine
        .lua
        .load("return d.tbl.filter(function(v) return v % 2 == 0 end, { 1, 2, 3, 4, 5, 6 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(filtered.raw_len(), 3);
    assert_eq!(filtered.get::<i64>(1).unwrap(), 2);
    assert_eq!(filtered.get::<i64>(2).unwrap(), 4);
    assert_eq!(filtered.get::<i64>(3).unwrap(), 6);

    let filtered_empty: Table = engine
        .lua
        .load("return d.tbl.filter(function(v) return v > 100 end, { 1, 2, 3 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(filtered_empty.raw_len(), 0);

    let mapped: Table = engine
        .lua
        .load("return d.tbl.map(function(v) return v * 10 end, { a = 1, b = 2 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(mapped.get::<i64>("a").unwrap(), 10);
    assert_eq!(mapped.get::<i64>("b").unwrap(), 20);

    let mapped_nil_removes: Table = engine
        .lua
        .load("return d.tbl.map(function(v) if v == 'drop' then return nil else return v end end, { a = 'keep', b = 'drop' })")
        .eval()
        .expect("Execution failed");
    assert_eq!(mapped_nil_removes.get::<String>("a").unwrap(), "keep");
    assert!(
        mapped_nil_removes
            .get::<Option<String>>("b")
            .unwrap()
            .is_none()
    );

    let invalid_filter_fn = engine
        .lua
        .load("return d.tbl.filter('not_fn', { 1, 2 })")
        .eval::<Table>();
    assert!(invalid_filter_fn.is_err());

    let invalid_filter_tbl = engine
        .lua
        .load("return d.tbl.filter(function() return true end, 123)")
        .eval::<Table>();
    assert!(invalid_filter_tbl.is_err());

    let invalid_map_fn = engine
        .lua
        .load("return d.tbl.map('not_fn', { a = 1 })")
        .eval::<Table>();
    assert!(invalid_map_fn.is_err());

    let invalid_map_tbl = engine
        .lua
        .load("return d.tbl.map(function(v) return v end, 'bad')")
        .eval::<Table>();
    assert!(invalid_map_tbl.is_err());
}

/// Verify that `d.tbl.keys`, `d.tbl.values`, `d.tbl.count`, and `d.tbl.isempty` inspect tables accurately.
#[test]
fn test_d_tbl_inspection() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let count_val: i64 = engine
        .lua
        .load("return d.tbl.count({ x = 1, y = 2, 100 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(count_val, 3);

    let count_empty: i64 = engine
        .lua
        .load("return d.tbl.count({})")
        .eval()
        .expect("Execution failed");
    assert_eq!(count_empty, 0);

    let is_empty_true: bool = engine
        .lua
        .load("return d.tbl.isempty({})")
        .eval()
        .expect("Execution failed");
    assert!(is_empty_true);

    let is_empty_false: bool = engine
        .lua
        .load("return d.tbl.isempty({ key = 'value' })")
        .eval()
        .expect("Execution failed");
    assert!(!is_empty_false);

    let is_empty_non_table: bool = engine
        .lua
        .load("return d.tbl.isempty('string')")
        .eval()
        .expect("Execution failed");
    assert!(!is_empty_non_table);

    let keys: Table = engine
        .lua
        .load("return d.tbl.keys({ first = 1, second = 2 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(keys.raw_len(), 2);

    let values: Table = engine
        .lua
        .load("return d.tbl.values({ a = 10, b = 20 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(values.raw_len(), 2);

    let count_err = engine
        .lua
        .load("return d.tbl.count('not_a_table')")
        .eval::<i64>();
    assert!(count_err.is_err());

    let keys_err = engine.lua.load("return d.tbl.keys(12345)").eval::<Table>();
    assert!(keys_err.is_err());

    let values_err = engine.lua.load("return d.tbl.values(nil)").eval::<Table>();
    assert!(values_err.is_err());
}

/// Verify that `d.tbl.isarray` and `d.tbl.islist` validate sequential integer keys.
#[test]
fn test_d_tbl_isarray_and_islist() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let array_valid: bool = engine
        .lua
        .load("return d.tbl.isarray({ 'a', 'b', 'c' })")
        .eval()
        .expect("Execution failed");
    assert!(array_valid);

    let list_empty: bool = engine
        .lua
        .load("return d.tbl.islist({})")
        .eval()
        .expect("Execution failed");
    assert!(list_empty);

    let single_item: bool = engine
        .lua
        .load("return d.tbl.islist({ 'single' })")
        .eval()
        .expect("Execution failed");
    assert!(single_item);

    let list_gap: bool = engine
        .lua
        .load("return d.tbl.islist({ [1] = 'a', [3] = 'c' })")
        .eval()
        .expect("Execution failed");
    assert!(!list_gap);

    let array_gap: bool = engine
        .lua
        .load("return d.tbl.isarray({ [1] = 'a', [3] = 'c' })")
        .eval()
        .expect("Execution failed");
    assert!(array_gap);

    let array_zero_index: bool = engine
        .lua
        .load("return d.tbl.islist({ [0] = 'a', [1] = 'b' })")
        .eval()
        .expect("Execution failed");
    assert!(!array_zero_index);

    let array_hash: bool = engine
        .lua
        .load("return d.tbl.islist({ key = 'val' })")
        .eval()
        .expect("Execution failed");
    assert!(!array_hash);

    let array_mixed: bool = engine
        .lua
        .load("return d.tbl.isarray({ 1, 2, tag = 'value' })")
        .eval()
        .expect("Execution failed");
    assert!(!array_mixed);

    let array_non_table: bool = engine
        .lua
        .load("return d.tbl.isarray(12345)")
        .eval()
        .expect("Execution failed");
    assert!(!array_non_table);

    let list_nil: bool = engine
        .lua
        .load("return d.tbl.islist(nil)")
        .eval()
        .expect("Execution failed");
    assert!(!list_nil);
}

/// Verify that `d.tbl.flatten` unrolls nested lists.
#[test]
fn test_d_tbl_flatten() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let full_flatten: Table = engine
        .lua
        .load("return d.tbl.flatten({ 1, { 2, { 3, 4 } }, 5 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(full_flatten.raw_len(), 5);
    assert_eq!(full_flatten.get::<i64>(1).unwrap(), 1);
    assert_eq!(full_flatten.get::<i64>(2).unwrap(), 2);
    assert_eq!(full_flatten.get::<i64>(3).unwrap(), 3);
    assert_eq!(full_flatten.get::<i64>(4).unwrap(), 4);
    assert_eq!(full_flatten.get::<i64>(5).unwrap(), 5);

    let empty_subtables: Table = engine
        .lua
        .load("return d.tbl.flatten({ 1, {}, 2, { {} }, 3 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_subtables.raw_len(), 3);
    assert_eq!(empty_subtables.get::<i64>(1).unwrap(), 1);
    assert_eq!(empty_subtables.get::<i64>(2).unwrap(), 2);
    assert_eq!(empty_subtables.get::<i64>(3).unwrap(), 3);

    let non_table_err = engine.lua.load("return d.tbl.flatten(123)").eval::<Table>();
    assert!(non_table_err.is_err());
}

/// Verify that `d.tbl.get` reads nested paths safely and returns nil on missing keys.
#[test]
fn test_d_tbl_get() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local data = {
            album = {
                info = { total_discs = 2, is_virtual = false },
                tracks = { { title = "Xtal" }, { title = "Tha" } }
            },
            empty_str = "",
            zero_val = 0
        }
        return {
            valid = d.tbl.get(data, 'album', 'info', 'total_discs'),
            false_val = d.tbl.get(data, 'album', 'info', 'is_virtual'),
            empty_str = d.tbl.get(data, 'empty_str'),
            zero_val = d.tbl.get(data, 'zero_val'),
            array_idx = d.tbl.get(data, 'album', 'tracks', 2, 'title'),
            missing = d.tbl.get(data, 'album', 'missing', 'field'),
            non_table = d.tbl.get(data, 'album', 'info', 'total_discs', 'deeper'),
            no_args = d.tbl.get(data),
            nil_root = d.tbl.get(nil, 'album')
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(res.get::<i64>("valid").unwrap(), 2);
    assert!(!res.get::<bool>("false_val").unwrap());
    assert_eq!(res.get::<String>("empty_str").unwrap(), "");
    assert_eq!(res.get::<i64>("zero_val").unwrap(), 0);
    assert_eq!(res.get::<String>("array_idx").unwrap(), "Tha");
    assert!(res.get::<Option<i64>>("missing").unwrap().is_none());
    assert!(res.get::<Option<i64>>("non_table").unwrap().is_none());
    assert!(res.get::<Option<i64>>("no_args").unwrap().is_none());
    assert!(res.get::<Option<i64>>("nil_root").unwrap().is_none());
}

/// Verify that `d.tbl.add_reverse_lookup` creates inverse mappings and rejects key collisions.
#[test]
fn test_d_tbl_add_reverse_lookup() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let mapped: Table = engine
        .lua
        .load("return d.tbl.add_reverse_lookup({ a = 1, b = 2 })")
        .eval()
        .expect("Execution failed");
    assert_eq!(mapped.get::<String>(1).unwrap(), "a");
    assert_eq!(mapped.get::<String>(2).unwrap(), "b");

    let self_mapped: Table = engine
        .lua
        .load("return d.tbl.add_reverse_lookup({ same = 'same' })")
        .eval()
        .expect("Execution failed");
    assert_eq!(self_mapped.get::<String>("same").unwrap(), "same");

    let collision_err = engine
        .lua
        .load("return d.tbl.add_reverse_lookup({ a = 1, [1] = 'other' })")
        .eval::<Table>();
    assert!(collision_err.is_err());

    let non_table_err = engine
        .lua
        .load("return d.tbl.add_reverse_lookup('bad_target')")
        .eval::<Table>();
    assert!(non_table_err.is_err());
}

/// Verify that `d.tbl.deepcopy` and `d.tbl.deep_equal` duplicate and compare nested values.
#[test]
fn test_d_tbl_deepcopy_and_deep_equal() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local orig = { a = 1, nested = { b = 2 } }
        local copy = d.tbl.deepcopy(orig)
        copy.nested.b = 99
        return {
            orig_b = orig.nested.b,
            copy_b = copy.nested.b,
            equal_same = d.tbl.deep_equal(orig, { a = 1, nested = { b = 2 } }),
            equal_diff = d.tbl.deep_equal(orig, copy),
            equal_prim = d.tbl.deep_equal(100, 100),
            diff_prim = d.tbl.deep_equal(100, 200),
            diff_types = d.tbl.deep_equal({ a = 1 }, 'string')
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(res.get::<i64>("orig_b").unwrap(), 2);
    assert_eq!(res.get::<i64>("copy_b").unwrap(), 99);
    assert!(res.get::<bool>("equal_same").unwrap());
    assert!(!res.get::<bool>("equal_diff").unwrap());
    assert!(res.get::<bool>("equal_prim").unwrap());
    assert!(!res.get::<bool>("diff_prim").unwrap());
    assert!(!res.get::<bool>("diff_types").unwrap());

    let cycle_code = r#"
        local a = { name = 'cycle' }
        a.self = a
        local b = d.tbl.deepcopy(a)
        return b.self == b
    "#;
    let cycle_preserved: bool = engine
        .lua
        .load(cycle_code)
        .eval()
        .expect("Execution failed");
    assert!(cycle_preserved);

    let primitive_copy: i64 = engine
        .lua
        .load("return d.tbl.deepcopy(42)")
        .eval()
        .expect("Execution failed");
    assert_eq!(primitive_copy, 42);

    let mt_code = r#"
        local proto = { kind = "prototype" }
        local t = setmetatable({ a = 1 }, proto)
        local copy = d.tbl.deepcopy(t)
        return getmetatable(copy) == proto
    "#;
    let mt_preserved: bool = engine.lua.load(mt_code).eval().expect("Execution failed");
    assert!(mt_preserved);

    let protected_mt_code = r#"
        local t = setmetatable({ a = 10 }, { __metatable = "protected_string" })
        local copy = d.tbl.deepcopy(t)
        return copy.a
    "#;
    let copy_a: i64 = engine
        .lua
        .load(protected_mt_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(copy_a, 10);
}

/// Verify index_by and group_by accept field names or functions and skip nil keys.
#[test]
fn test_d_tbl_index_by_and_group_by() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        local items = {
            { id = 'a', kind = 'x' },
            { id = 'b', kind = 'x' },
            { id = 'c', kind = 'y' },
            { kind = 'z' },
        }
        local by_id = d.tbl.index_by(items, 'id')
        local by_kind = d.tbl.group_by(items, 'kind')
        local by_fn = d.tbl.index_by(items, function(item) return item.id end)
        return {
            a = by_id.a.kind,
            by_id_count = d.tbl.count(by_id),
            x_count = #by_kind.x,
            y_count = #by_kind.y,
            z_count = #by_kind.z,
            fn_count = d.tbl.count(by_fn),
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("index/group");

    assert_eq!(res.get::<String>("a").unwrap(), "x");
    assert_eq!(res.get::<i64>("by_id_count").unwrap(), 3);
    assert_eq!(res.get::<i64>("x_count").unwrap(), 2);
    assert_eq!(res.get::<i64>("y_count").unwrap(), 1);
    assert_eq!(res.get::<i64>("z_count").unwrap(), 1);
    assert_eq!(res.get::<i64>("fn_count").unwrap(), 3);

    let invalid = engine
        .lua
        .load("return d.tbl.index_by({ { id = 1 } }, 123)")
        .eval::<Table>();
    assert!(invalid.is_err());
}
