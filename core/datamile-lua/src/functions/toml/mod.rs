#[cfg(test)]
mod tests;

use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};

pub fn register(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    let toml_table = lua.create_table()?;
    toml_table.set(
        "decode",
        lua.create_function(move |lua, s: String| {
            let toml_val: toml::Value =
                toml::from_str(&s).map_err(mlua::Error::external)?;
            let json_val = datamile_fs::toml_to_json(toml_val);
            lua.to_value_with(&json_val, opts)
        })?,
    )?;

    toml_table.set(
        "encode",
        lua.create_function(|lua, val: mlua::Value| {
            let json_val: serde_json::Value = lua.from_value(val)?;
            let toml_val = datamile_fs::json_to_toml(json_val);
            toml::to_string_pretty(&toml_val).map_err(mlua::Error::external)
        })?,
    )?;

    datamile_tbl.set("toml", toml_table)
}
