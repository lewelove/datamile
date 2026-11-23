#[cfg(test)]
mod tests;

use mlua::{Lua, Table};

const LUA_TBL: &str = include_str!("mod.lua");

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let tbl_table: Table = lua.load(LUA_TBL).eval()?;
    datamile_tbl.set("tbl", tbl_table)?;
    Ok(())
}
