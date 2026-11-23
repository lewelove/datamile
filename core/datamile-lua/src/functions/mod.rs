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

pub fn register_all(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    r#fn::register(lua, datamile_tbl)?;
    fs::register(lua, datamile_tbl, opts)?;
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
