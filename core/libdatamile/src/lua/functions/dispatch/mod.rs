//! `d.dispatch.<command>(args, opts)` runs the current Datamile binary for a
//! CLI subcommand, forwarding the active `--config`/`--media-directory` flags.

use mlua::{Lua, Table, Value};

use super::system::lua_system;

/// Subcommands exposed to Lua; anything else is a typo or an internal path.
const COMMANDS: &[&str] = &["compile", "update", "manifest", "harvest", "query", "x"];

fn global_flags() -> Vec<String> {
    let mut flags = Vec::new();

    if let Some(config) = crate::lua::resolve_config_path() {
        flags.push("--config".to_string());
        flags.push(config.to_string_lossy().into_owned());
    }

    if let Some(dir) = crate::lua::media_directory_override() {
        flags.push("--media-directory".to_string());
        flags.push(dir.to_string_lossy().into_owned());
    }

    flags
}

fn arg_strings(value: Value) -> mlua::Result<Vec<String>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::String(s) => Ok(vec![s.to_str()?.to_string()]),
        Value::Table(tbl) => tbl.sequence_values::<String>().collect(),
        _ => Err(mlua::Error::runtime(
            "dispatch args must be a table of strings or a string",
        )),
    }
}

fn dispatch(
    command: &str,
    args: Value,
    opts: Option<Table>,
    lua: &Lua,
) -> mlua::Result<Value> {
    let exe = std::env::current_exe().map_err(mlua::Error::external)?;
    let full_command: Vec<String> = std::iter::once(exe.to_string_lossy().into_owned())
        .chain(global_flags())
        .chain(std::iter::once(command.to_string()))
        .chain(arg_strings(args)?)
        .collect();

    let table = lua.create_table()?;
    for (index, arg) in full_command.into_iter().enumerate() {
        table.set(index + 1, arg)?;
    }

    // Default to inherited stdio so the user sees subcommand progress.
    let opts = if let Some(opts) = opts {
        opts
    } else {
        let opts = lua.create_table()?;
        opts.set("stdio", "inherit")?;
        opts
    };

    lua_system(lua, (Value::Table(table), Some(opts)))
}

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let dispatch_tbl = lua.create_table()?;

    for command in COMMANDS {
        let name = (*command).to_string();
        let handler = name.clone();
        let func =
            lua.create_function(move |lua, (args, opts): (Value, Option<Table>)| {
                dispatch(&handler, args, opts, lua)
            })?;
        dispatch_tbl.set(name, func)?;
    }

    datamile_tbl.set("dispatch", dispatch_tbl)
}

#[cfg(test)]
mod tests;
