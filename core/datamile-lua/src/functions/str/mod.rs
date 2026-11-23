#[cfg(test)]
mod tests;

use mlua::{Lua, Table};

const LUA_STR: &str = include_str!("mod.lua");

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let str_table: Table = lua.load(LUA_STR).eval()?;
    datamile_tbl.set("str", str_table)
}
