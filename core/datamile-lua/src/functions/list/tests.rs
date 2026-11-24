use mlua::Table;

use crate::LuaRuntime;

/// `d.list.sum` adds numbers directly or through a mapper.
#[test]
fn test_d_list_sum() {
    let engine = LuaRuntime::new().expect("runtime");

    let plain: i64 = engine
        .lua
        .load("return d.list.sum({ 1, 2, 3 })")
        .eval()
        .expect("sum");
    assert_eq!(plain, 6);

    let mapped: i64 = engine
        .lua
        .load("return d.list.sum({ { n = 2 }, { n = 3 } }, function(x) return x.n end)")
        .eval()
        .expect("sum with fn");
    assert_eq!(mapped, 5);

    let empty: i64 = engine
        .lua
        .load("return d.list.sum({})")
        .eval()
        .expect("empty");
    assert_eq!(empty, 0);

    let bad = engine
        .lua
        .load("return d.list.sum({ 'x' })")
        .eval::<Table>();
    assert!(bad.is_err());
}
