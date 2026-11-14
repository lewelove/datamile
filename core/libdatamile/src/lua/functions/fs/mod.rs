#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use mlua::serde::SerializeOptions;
use mlua::{Lua, LuaSerdeExt, Table};

use crate::error::DatamileError;
use crate::lua::EngineContext;

const LUA_FS: &str = include_str!("mod.lua");

fn parse_path_with_ctx(ctx: &EngineContext, path_str: Option<String>) -> Option<PathBuf> {
    let raw_path = path_str?;
    if raw_path.trim().is_empty() {
        return None;
    }
    let path = crate::utils::expand_path(&raw_path);
    ctx.record_dependency(&path);
    Some(path)
}

fn parse_and_record_path(lua: &Lua, path_str: Option<String>) -> Option<PathBuf> {
    let ctx = lua.app_data_ref::<EngineContext>()?;
    parse_path_with_ctx(&ctx, path_str)
}

fn fs_exists(lua: &Lua, path_str: Option<String>) -> bool {
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return false;
    };
    path.exists()
}

fn fs_read(lua: &Lua, path_str: Option<String>) -> mlua::Result<Option<String>> {
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(mlua::Error::external(err)),
    }
}

fn fs_read_lines(lua: &Lua, path_str: Option<String>) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return Ok(table);
    };

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(table),
        Err(err) => return Err(mlua::Error::external(err)),
    };

    let reader = BufReader::new(file);
    let mut idx = 1;
    for line_res in reader.lines() {
        let line = line_res.map_err(mlua::Error::external)?;
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            table.set(idx, trimmed)?;
            table.set(trimmed, idx)?;
            idx += 1;
        }
    }

    Ok(table)
}

fn fs_read_dotenv(lua: &Lua, path_str: Option<String>) -> mlua::Result<Option<Table>> {
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return Ok(None);
    };

    let loader = dotenv_ng_core::EnvLoader::with_path(&path)
        .sequence(dotenv_ng_core::EnvSequence::InputOnly)
        .required(true);

    match loader.load() {
        Ok(env_map) => {
            let table = lua.create_table()?;
            for (k, v) in env_map {
                table.set(k, v)?;
            }
            Ok(Some(table))
        }
        Err(err) if err.not_found() => Ok(None),
        Err(err) => Err(mlua::Error::external(err)),
    }
}

fn fs_scandir_native(
    lua: &Lua,
    (path_str, follow): (String, bool),
) -> mlua::Result<mlua::Function> {
    let expanded = crate::utils::expand_path(&path_str);
    if let Some(ctx) = lua.app_data_ref::<EngineContext>() {
        ctx.record_dir(std::path::Path::new(&expanded));
    }

    // Directory handles are read eagerly and dropped here: keeping a live
    // `ReadDir` inside the returned Lua iterator leaks one file descriptor per
    // visited directory until garbage collection runs.
    let mut entries: Vec<(String, &'static str)> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(&expanded) {
        for entry in read_dir.flatten() {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy().into_owned();
            let ft = entry.file_type().ok();

            let is_symlink = ft.as_ref().is_some_and(std::fs::FileType::is_symlink);
            let resolved_ft = if follow && is_symlink {
                std::fs::metadata(entry.path()).map(|m| m.file_type()).ok()
            } else {
                ft
            };

            let type_str = if resolved_ft.as_ref().is_some_and(std::fs::FileType::is_file)
            {
                "file"
            } else if resolved_ft.as_ref().is_some_and(std::fs::FileType::is_dir) {
                "directory"
            } else if is_symlink {
                "link"
            } else {
                "unknown"
            };

            entries.push((file_name_str, type_str));
        }
    }

    let mut index = 0;
    lua.create_function_mut(move |_, ()| {
        let Some((name, type_str)) = entries.get(index) else {
            return Ok((None::<String>, None::<&str>));
        };
        index += 1;

        Ok((Some(name.clone()), Some(*type_str)))
    })
}

fn fs_read_audio(lua: &Lua, path_str: Option<String>) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return Ok(out);
    };

    let cache_root = lua.app_data_ref::<EngineContext>().map_or_else(
        || crate::utils::expand_path("~/.cache/datamile"),
        |c| c.cache_root.clone(),
    );
    let track = crate::harvest::harvest_file_cached(&path, &cache_root)
        .map_err(mlua::Error::external)?;

    out.set("path", path.to_string_lossy().to_string())?;
    out.set("duration_milliseconds", track.physics.duration_ms)?;
    out.set("sample_rate", track.physics.sample_rate)?;
    out.set("bit_depth", track.physics.bit_depth)?;
    out.set("channels", track.physics.channels)?;
    out.set("audio_bitrate", track.physics.audio_bitrate)?;
    out.set("overall_bitrate", track.physics.overall_bitrate)?;
    out.set("format", track.physics.format)?;
    out.set("mtime", track.physics.mtime)?;
    out.set("byte_size", track.physics.file_size)?;

    let tags = lua.create_table()?;
    for (key, value) in track.tags {
        tags.set(key, lua.to_value(&value)?)?;
    }
    out.set("tags", tags)?;

    if let Some(chapters) = track.chapters {
        let list = lua.create_table()?;
        for (index, chapter) in chapters.into_iter().enumerate() {
            let item = lua.create_table()?;
            item.set("title", chapter.title)?;
            item.set("start_ms", chapter.start_ms)?;
            item.set("end_ms", chapter.end_ms)?;
            list.set(index + 1, item)?;
        }
        out.set("chapters", list)?;
    }

    Ok(out)
}

fn fs_stat(lua: &Lua, path_str: Option<String>) -> mlua::Result<Option<Table>> {
    let Some(path) = parse_and_record_path(lua, path_str) else {
        return Ok(None);
    };

    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(mlua::Error::external(err)),
    };
    let out = lua.create_table()?;
    let mtime = meta
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    out.set("path", path.to_string_lossy().to_string())?;
    out.set("mtime", mtime)?;
    out.set("byte_size", meta.len())?;
    out.set("is_file", meta.is_file())?;
    out.set("is_dir", meta.is_dir())?;
    Ok(Some(out))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "mlua callbacks must return Result"
)]
fn fs_ext(_: &Lua, path: Option<String>) -> mlua::Result<Option<String>> {
    Ok(path.and_then(|path| {
        std::path::Path::new(&path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
    }))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "mlua callbacks must return Result"
)]
fn fs_stem(_: &Lua, path: Option<String>) -> mlua::Result<Option<String>> {
    Ok(path.and_then(|path| {
        std::path::Path::new(&path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(ToString::to_string)
    }))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "mlua callbacks must return Result"
)]
fn fs_is_audio(
    _: &Lua,
    (path, exts): (String, Option<Vec<String>>),
) -> mlua::Result<bool> {
    let Some(ext) = std::path::Path::new(&path)
        .extension()
        .and_then(|ext| ext.to_str())
    else {
        return Ok(false);
    };

    Ok(exts.map_or_else(
        || {
            crate::harvest::is_matching_extension(
                ext,
                crate::harvest::SUPPORTED_AUDIO_EXTENSIONS,
            )
        },
        |exts| crate::harvest::is_matching_extension(ext, &exts),
    ))
}

fn create_fs_reader(
    lua: &Lua,
    opts: SerializeOptions,
    expected_ext: &'static str,
) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, path_str: Option<String>| {
        let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
            return Err(mlua::Error::external(anyhow::anyhow!(
                "EngineContext is not initialized"
            )));
        };
        let Some(path) = parse_path_with_ctx(&ctx, path_str) else {
            return Ok(mlua::Value::Nil);
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case(expected_ext) {
            return Err(mlua::Error::external(DatamileError::InvalidFileExtension {
                path,
                expected: expected_ext.to_string(),
            }));
        }
        match crate::cache::read_object_cached(&path, &ctx.cache_root) {
            Ok(json_val) => lua.to_value_with(&json_val, opts),
            Err(DatamileError::ManifestIoError(err))
                if err.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(mlua::Value::Nil)
            }
            Err(e) => Err(mlua::Error::external(e)),
        }
    })
}

pub fn register(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    let fs_table: Table = lua.load(LUA_FS).eval()?;
    fs_table.set(
        "exists",
        lua.create_function(|lua, path_str: Option<String>| {
            Ok(fs_exists(lua, path_str))
        })?,
    )?;
    fs_table.set(
        "read",
        lua.create_function(|lua, path_str: Option<String>| fs_read(lua, path_str))?,
    )?;
    fs_table.set(
        "read_lines",
        lua.create_function(|lua, path_str: Option<String>| {
            fs_read_lines(lua, path_str)
        })?,
    )?;
    fs_table.set(
        "read_dotenv",
        lua.create_function(|lua, path_str: Option<String>| {
            fs_read_dotenv(lua, path_str)
        })?,
    )?;
    fs_table.set(
        "read_audio",
        lua.create_function(|lua, path_str: Option<String>| {
            fs_read_audio(lua, path_str)
        })?,
    )?;
    fs_table.set(
        "stat",
        lua.create_function(|lua, path_str: Option<String>| fs_stat(lua, path_str))?,
    )?;
    fs_table.set("ext", lua.create_function(fs_ext)?)?;
    fs_table.set("stem", lua.create_function(fs_stem)?)?;
    fs_table.set("is_audio", lua.create_function(fs_is_audio)?)?;
    fs_table.set(
        "audio_extensions",
        lua.create_function(|lua, ()| {
            let exts = lua.create_table()?;
            for ext in crate::harvest::SUPPORTED_AUDIO_EXTENSIONS {
                exts.push(*ext)?;
            }
            Ok(exts)
        })?,
    )?;
    fs_table.set("read_json", create_fs_reader(lua, opts, "json")?)?;
    fs_table.set("read_toml", create_fs_reader(lua, opts, "toml")?)?;
    fs_table.set(
        "_scandir_native",
        lua.create_function(|lua, args: (String, Option<bool>)| {
            fs_scandir_native(lua, (args.0, args.1.unwrap_or(false)))
        })?,
    )?;

    datamile_tbl.set("fs", fs_table)
}
