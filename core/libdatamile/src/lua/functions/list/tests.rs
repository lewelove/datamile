use mlua::Table;

use crate::lua::LuaEngine;

/// Verify sort_by sorts scalar and tuple keys in place with natural string order.
#[test]
fn test_d_list_sort_by() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        local items = { { name = 'ep10' }, { name = 'ep2' }, { name = 'ep1' } }
        local same = d.list.sort_by(items, function(item) return item.name end)
        local out = {}
        for _, item in ipairs(same) do
            table.insert(out, item.name)
        end
        return out
    "#;
    let names: Vec<String> = engine.lua.load(code).eval().expect("sort_by scalar");
    assert_eq!(names, vec!["ep1", "ep2", "ep10"]);

    let code = r#"
        local items = {
            { disc = 2, track = 1 },
            { disc = 1, track = 10 },
            { disc = 1, track = 2 },
        }
        local out = {}
        for _, item in ipairs(d.list.sort_by(items, function(item)
            return { item.disc, item.track }
        end)) do
            table.insert(out, item.disc .. ':' .. item.track)
        end
        return out
    "#;
    let tuples: Vec<String> = engine.lua.load(code).eval().expect("sort_by tuple");
    assert_eq!(tuples, vec!["1:2", "1:10", "2:1"]);

    let invalid = engine
        .lua
        .load("return d.list.sort_by({ 1, 2 }, 'nope')")
        .eval::<Table>();
    assert!(invalid.is_err());
}

/// Verify sort_by evaluates the key function once per item, not per comparison.
#[test]
fn test_sort_by_calls_key_once() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        local calls = 0
        local items = { { v = 3 }, { v = 1 }, { v = 2 } }
        d.list.sort_by(items, function(item)
            calls = calls + 1
            return item.v
        end)
        return calls
    "#;
    let calls: usize = engine.lua.load(code).eval().expect("sort_by calls");
    assert_eq!(calls, 3);
}

/// Verify sum, min and max map values and ignore nil mapped entries.
#[test]
fn test_d_list_sum_min_max() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        local items = { { v = 2 }, { v = nil }, { v = 5 } }
        return {
            sum = d.list.sum(items, function(item) return item.v end),
            sum_plain = d.list.sum({ 1, 2, 3 }),
            empty = d.list.sum({}),
            min = d.list.min(items, function(item) return item.v end),
            max = d.list.max(items, function(item) return item.v end),
            min_plain = d.list.min({ 3, 1, 2 }),
            max_plain = d.list.max({ 3, 1, 2 }),
            empty_min = d.list.min({}),
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("sum/min/max");

    assert_eq!(res.get::<i64>("sum").unwrap(), 7);
    assert_eq!(res.get::<i64>("sum_plain").unwrap(), 6);
    assert_eq!(res.get::<i64>("empty").unwrap(), 0);
    assert_eq!(res.get::<i64>("min").unwrap(), 2);
    assert_eq!(res.get::<i64>("max").unwrap(), 5);
    assert_eq!(res.get::<i64>("min_plain").unwrap(), 1);
    assert_eq!(res.get::<i64>("max_plain").unwrap(), 3);
    assert!(res.get::<Option<i64>>("empty_min").unwrap().is_none());
}

/// Verify all and any over empty and non-empty lists.
#[test]
fn test_d_list_all_and_any() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        return {
            all_true = d.list.all({ 1, 2, 3 }, function(v) return v > 0 end),
            all_false = d.list.all({ 1, 0, 3 }, function(v) return v > 0 end),
            all_empty = d.list.all({}, function() return false end),
            any_true = d.list.any({ 1, 2, 3 }, function(v) return v == 2 end),
            any_false = d.list.any({ 1, 2, 3 }, function(v) return v > 9 end),
            any_empty = d.list.any({}, function() return true end),
            indexed = d.list.all({ 10, 20 }, function(v, i) return v == i * 10 end),
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("all/any");

    assert!(res.get::<bool>("all_true").unwrap());
    assert!(!res.get::<bool>("all_false").unwrap());
    assert!(res.get::<bool>("all_empty").unwrap());
    assert!(res.get::<bool>("any_true").unwrap());
    assert!(!res.get::<bool>("any_false").unwrap());
    assert!(!res.get::<bool>("any_empty").unwrap());
    assert!(res.get::<bool>("indexed").unwrap());
}
