#[cfg(test)]
mod tests;

use mlua::{Lua, Table};

const LUA_FMT: &str = include_str!("mod.lua");

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let fmt_table: Table = lua.load(LUA_FMT).eval()?;
    datamile_tbl.set("fmt", fmt_table)
}
