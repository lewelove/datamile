#[cfg(test)]
mod tests;

use mlua::{Lua, Table};

const LUA_STR: &str = include_str!("mod.lua");

fn natcmp(_: &Lua, (a, b): (String, String)) -> mlua::Result<i32> {
    Ok(match alphanumeric_sort::compare_str(&a, &b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let str_table: Table = lua.load(LUA_STR).eval()?;
    str_table.set("natcmp", lua.create_function(natcmp)?)?;
    str_table.set(
        "url_encode",
        lua.create_function(|_, s: String| Ok(urlencoding::encode(&s).into_owned()))?,
    )?;
    datamile_tbl.set("str", str_table)
}
