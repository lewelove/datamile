#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use datamile_cache::stat::is_lock_file;
use mlua::serde::SerializeOptions;
use mlua::{Lua, Table};
use walkdir::WalkDir;

use crate::{EngineContext, read_with_entity_memo};

const LUA_FS: &str = include_str!("mod.lua");

fn parse_path_with_ctx(
    ctx: &EngineContext,
    path_str: Option<String>,
) -> mlua::Result<Option<PathBuf>> {
    let Some(raw_path) = path_str else {
        return Ok(None);
    };
    if raw_path.trim().is_empty() {
        return Ok(None);
    }
    let path = datamile_fs::expand_path(&raw_path);
    ctx.record_dependency(&path)?;

    Ok(Some(path))
}

fn parse_and_record_path(
    lua: &Lua,
    path_str: Option<String>,
) -> mlua::Result<Option<PathBuf>> {
    let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
        return Ok(None);
    };

    parse_path_with_ctx(&ctx, path_str)
}

/// Whether a compile is running, in which case lock files are outputs and must
/// stay out of every enumeration.
fn compiling(lua: &Lua) -> bool {
    lua.app_data_ref::<EngineContext>()
        .is_some_and(|ctx| ctx.is_compiling())
}

fn fs_exists(lua: &Lua, path_str: Option<String>) -> mlua::Result<bool> {
    let Some(path) = parse_and_record_path(lua, path_str)? else {
        return Ok(false);
    };

    Ok(path.exists())
}

fn fs_read(lua: &Lua, path_str: Option<String>) -> mlua::Result<Option<String>> {
    let Some(path) = parse_and_record_path(lua, path_str)? else {
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
    let Some(path) = parse_and_record_path(lua, path_str)? else {
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
    let Some(path) = parse_and_record_path(lua, path_str)? else {
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
    let expanded = datamile_fs::expand_path(&path_str);
    let hide_locks = compiling(lua);

    let Ok(mut read_dir) = std::fs::read_dir(&expanded) else {
        return lua.create_function(|_, ()| Ok((None::<String>, None::<&str>)));
    };

    lua.create_function_mut(move |_, ()| {
        for entry_res in read_dir.by_ref() {
            let Ok(entry) = entry_res else {
                continue;
            };
            if hide_locks && is_lock_file(&entry.path()) {
                continue;
            }

            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
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

            return Ok((Some(file_name_str.into_owned()), Some(type_str)));
        }
        Ok((None, None))
    })
}

fn create_fs_reader(
    lua: &Lua,
    _opts: SerializeOptions,
    expected_ext: &'static str,
) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, path_str: Option<String>| {
        let Some(ctx) = lua.app_data_ref::<EngineContext>() else {
            return Err(mlua::Error::external(anyhow::anyhow!(
                "EngineContext is not initialized"
            )));
        };
        let Some(path) = parse_path_with_ctx(&ctx, path_str)? else {
            return Ok(mlua::Value::Nil);
        };
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case(expected_ext) {
            return Err(mlua::Error::external(
                datamile_cache::CacheError::InvalidFileExtension {
                    path,
                    expected: expected_ext.to_string(),
                },
            ));
        }

        // A memo hit survives the file vanishing mid-entity; a fresh read of a
        // missing file is `nil`, matching `d.fs.read`.
        if let Some(value) = ctx.memo_get(expected_ext, &path) {
            return Ok(value);
        }
        if !path.exists() {
            return Ok(mlua::Value::Nil);
        }

        let cache_root = ctx.cache_root.clone();
        read_with_entity_memo(lua, expected_ext, &path, |p| {
            Ok(datamile_cache::read_object_cached(p, &cache_root)?)
        })
    })
}

fn meta_seconds(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    File,
    Dir,
}

fn entry_table(
    lua: &Lua,
    path: &Path,
    meta: &std::fs::Metadata,
    kind: EntryKind,
) -> mlua::Result<Table> {
    let entry = lua.create_table()?;
    entry.set("path", path.to_string_lossy().to_string())?;
    entry.set(
        "name",
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().to_string()),
    )?;
    entry.set("mtime", meta_seconds(meta))?;

    if kind == EntryKind::File {
        entry.set(
            "stem",
            path.file_stem()
                .map_or_else(String::new, |s| s.to_string_lossy().to_string()),
        )?;
        entry.set(
            "ext",
            path.extension()
                .map_or_else(String::new, |e| e.to_string_lossy().to_string()),
        )?;
        entry.set("byte_size", meta.len())?;
    }

    Ok(entry)
}

fn matches_ext(path: &Path, extensions: &[String]) -> bool {
    extensions.is_empty() || datamile_fs::has_extension(path, extensions)
}

fn sorted_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|a, b| alphanumeric_sort::compare_path(a, b));
}

/// Recursively collects every directory below `dir` (inclusive) holding at
/// least one of `markers`, canonicalized and naturally sorted.
fn fs_roots(lua: &Lua, (dir, markers): (String, Vec<String>)) -> mlua::Result<Table> {
    let root = datamile_fs::expand_path(&dir);
    let mut found: Vec<PathBuf> = WalkDir::new(&root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir())
        .filter(|entry| {
            markers
                .iter()
                .any(|marker| entry.path().join(marker).exists())
        })
        .map(|entry| {
            entry
                .path()
                .canonicalize()
                .unwrap_or_else(|_| entry.path().to_path_buf())
        })
        .collect();

    sorted_paths(&mut found);
    found.dedup();

    let table = lua.create_table()?;
    for (index, path) in found.iter().enumerate() {
        table.set(index + 1, path.to_string_lossy().to_string())?;
    }
    Ok(table)
}

/// Lists file entries with their stat data in one walk.
fn fs_files(
    lua: &Lua,
    (dir, recursive, extensions): (String, bool, Vec<String>),
) -> mlua::Result<Table> {
    let root = datamile_fs::expand_path(&dir);
    let walker = if recursive {
        WalkDir::new(&root).follow_links(true).into_iter()
    } else {
        WalkDir::new(&root)
            .max_depth(1)
            .follow_links(true)
            .into_iter()
    };

    let hide_locks = compiling(lua);
    let mut entries: Vec<(PathBuf, std::fs::Metadata)> = walker
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| !hide_locks || !is_lock_file(entry.path()))
        .filter(|entry| matches_ext(entry.path(), &extensions))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((entry.path().to_path_buf(), meta))
        })
        .collect();

    entries.sort_by(|(a, _), (b, _)| alphanumeric_sort::compare_path(a, b));

    let table = lua.create_table()?;
    for (index, (path, meta)) in entries.iter().enumerate() {
        table.set(index + 1, entry_table(lua, path, meta, EntryKind::File)?)?;
    }
    Ok(table)
}

/// Lists directory entries with their mtime in one walk.
fn fs_dirs(lua: &Lua, (dir, recursive): (String, bool)) -> mlua::Result<Table> {
    let root = datamile_fs::expand_path(&dir);
    let walker = if recursive {
        WalkDir::new(&root).follow_links(true).into_iter()
    } else {
        WalkDir::new(&root)
            .max_depth(1)
            .follow_links(true)
            .into_iter()
    };

    let mut entries: Vec<(PathBuf, std::fs::Metadata)> = walker
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_dir() && entry.path() != root)
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((entry.path().to_path_buf(), meta))
        })
        .collect();

    entries.sort_by(|(a, _), (b, _)| alphanumeric_sort::compare_path(a, b));

    let table = lua.create_table()?;
    for (index, (path, meta)) in entries.iter().enumerate() {
        table.set(index + 1, entry_table(lua, path, meta, EntryKind::Dir)?)?;
    }
    Ok(table)
}

/// First candidate that exists, or the first file with a bare extension.
fn fs_first(
    lua: &Lua,
    (dir, candidates): (String, Vec<String>),
) -> mlua::Result<mlua::Value> {
    let root = datamile_fs::expand_path(&dir);
    let hide_locks = compiling(lua);

    for candidate in &candidates {
        if candidate.trim().is_empty() {
            continue;
        }

        let is_bare_ext = (!candidate.contains('/') && !candidate.contains('.'))
            || candidate.starts_with('.');
        if is_bare_ext {
            let ext = candidate.trim_start_matches('.').to_string();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&root)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && (!hide_locks || !is_lock_file(path))
                        && datamile_fs::has_extension(path, std::slice::from_ref(&ext))
                })
                .collect();
            sorted_paths(&mut files);
            if let Some(first) = files.first() {
                return Ok(mlua::Value::String(
                    lua.create_string(first.to_string_lossy().as_bytes())?,
                ));
            }
            continue;
        }

        let path = if Path::new(candidate).is_absolute() {
            PathBuf::from(candidate)
        } else {
            root.join(candidate)
        };
        if path.exists() && !(hide_locks && is_lock_file(&path)) {
            return Ok(mlua::Value::String(
                lua.create_string(path.to_string_lossy().as_bytes())?,
            ));
        }
    }

    Ok(mlua::Value::Nil)
}

pub fn register(
    lua: &Lua,
    datamile_tbl: &Table,
    opts: SerializeOptions,
) -> mlua::Result<()> {
    let fs_table: Table = lua.load(LUA_FS).eval()?;
    fs_table.set(
        "exists",
        lua.create_function(|lua, path_str: Option<String>| fs_exists(lua, path_str))?,
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
    fs_table.set("read_json", create_fs_reader(lua, opts, "json")?)?;
    fs_table.set("read_toml", create_fs_reader(lua, opts, "toml")?)?;
    fs_table.set(
        "_scandir_native",
        lua.create_function(|lua, args: (String, Option<bool>)| {
            fs_scandir_native(lua, (args.0, args.1.unwrap_or(false)))
        })?,
    )?;
    fs_table.set("_roots_native", lua.create_function(fs_roots)?)?;
    fs_table.set("_files_native", lua.create_function(fs_files)?)?;
    fs_table.set("_dirs_native", lua.create_function(fs_dirs)?)?;
    fs_table.set("_first_native", lua.create_function(fs_first)?)?;

    datamile_tbl.set("fs", fs_table)
}
