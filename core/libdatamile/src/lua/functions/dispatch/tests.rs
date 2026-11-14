use mlua::{Lua, Value};

use super::{COMMANDS, arg_strings, register};

/// Every whitelisted subcommand is exposed as a callable.
#[test]
fn registers_one_function_per_command() {
    let lua = Lua::new();
    let tbl = lua.create_table().expect("table");
    register(&lua, &tbl).expect("register");

    let dispatch: mlua::Table = tbl.get("dispatch").expect("dispatch");
    for command in COMMANDS {
        let _: mlua::Function = dispatch.get(*command).expect(command);
    }
}

/// Args accept both a sequence table and a bare string.
#[test]
fn args_accept_table_and_string() {
    let lua = Lua::new();

    let tbl = lua.create_table().expect("table");
    tbl.set(1, "a").expect("set");
    tbl.set(2, "b").expect("set");
    let args = arg_strings(Value::Table(tbl)).expect("args");
    assert_eq!(args, vec!["a".to_string(), "b".to_string()]);

    let single = lua.create_string("x").expect("string");
    let args = arg_strings(Value::String(single)).expect("args");
    assert_eq!(args, vec!["x".to_string()]);
}
