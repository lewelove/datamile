pub mod fmt;
pub mod r#fn;
pub mod fs;
pub mod get;
pub mod iter;
pub mod json;
pub mod list;
pub mod str;
pub mod system;
pub mod tbl;
pub mod toml;

use mlua::serde::SerializeOptions;
use mlua::{Lua, Table};
use serde_json::Value;

/// Sorts every object key in `value` so encoded output is byte-stable: Lua
/// iterates table pairs in hash order, which differs between VMs.
pub(crate) fn sort_json_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for v in map.values_mut() {
                sort_json_keys(v);
            }
            map.sort_keys();
        }
        Value::Array(items) => {
            for v in items {
                sort_json_keys(v);
            }
        }
        _ => {}
    }
}

pub fn register_all(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    r#fn::register(lua, datamile_tbl)?;
    fs::register(lua, datamile_tbl, opts)?;
    fmt::register(lua, datamile_tbl)?;
    get::register(lua, datamile_tbl)?;
    iter::register(lua, datamile_tbl)?;
    list::register(lua, datamile_tbl)?;
    system::register(lua, datamile_tbl)?;
    json::register(lua, datamile_tbl, opts)?;
    str::register(lua, datamile_tbl)?;
    tbl::register(lua, datamile_tbl)?;
    toml::register(lua, datamile_tbl, opts)?;
    Ok(())
}
