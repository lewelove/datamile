use std::fs;
use std::path::Path;

use libdatamile::lua::LuaEngine;
use serde_json::json;
use tempfile::tempdir;

use super::{RootTarget, compile_root};

fn root_target(root: &Path, root_type: &str, root_dir: &Path) -> RootTarget {
    RootTarget {
        root_type: root_type.to_string(),
        target: root.to_path_buf(),
        lock_path: root.join(format!("{root_type}.lock.json")),
        root_dir: root_dir.to_path_buf(),
        source: json!({ "path": root.to_string_lossy() }),
    }
}

/// Leaves finalize before their parents so parents aggregate compiled children
/// (3-level chain: root -> middle -> leaf).
#[test]
fn compile_root_runs_bottom_up() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("m1/l1")).expect("dirs");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "dir",
            select = function(parent) return { parent.path .. "/l1" } end,
        })
        d.compile.leaf.tag("name", function(ctx) return ctx.path:match("[^/]+$") end)
        d.compile.leaf.id(function(ctx) return "leaf:" .. ctx.tags.name end)

        d.media("middle", {
            kind = "dir",
            select = function(parent) return { parent.path .. "/m1" } end,
            children = { leaves = "leaf" },
        })
        d.compile.middle.id(function(ctx) return "middle:" .. #ctx.leaves end)
        d.compile.middle.object("info", function(ctx)
            return { leaf_id = ctx.leaves[1].id }
        end)

        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { middles = "middle" },
        })
        d.compile.root.id(function(ctx) return "root:" .. ctx.middles[1].id end)
        d.compile.root.object("info", function(ctx)
            return {
                middle_id = ctx.middles[1].id,
                leaf_id = ctx.middles[1].objects.info.leaf_id,
            }
        end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect("compile");

    let entity = &build.lock_json["root"];
    assert_eq!(entity["id"], "root:middle:1");
    assert_eq!(entity["objects"]["info"]["leaf_id"], "leaf:l1");
    assert_eq!(entity["objects"]["info"]["middle_id"], "middle:1");
    assert_eq!(entity["middles"][0]["leaves"][0]["tags"]["name"], "l1");
    assert_eq!(build.lock_path, root.join("root.lock.json"));
}

/// Each child type uses its own selector, which receives the parent ctx.
#[test]
fn compile_root_uses_child_type_select() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("items")).expect("dirs");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "dir",
            select = function(parent) return { parent.path .. "/items" } end,
        })
        d.compile.leaf.tag("kind", function() return "item" end)

        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { leaves = "leaf" },
        })
        d.compile.root.id(function() return "root" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect("compile");

    assert_eq!(build.lock_json["root"]["leaves"][0]["tags"]["kind"], "item");
}

/// The entity directory is always a dependency, so dropping a new file into
/// it later marks the root dirty even if no hook scanned the directory.
#[test]
fn compile_root_records_entity_directory() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("dirs");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
        })
        d.compile.root.id(function() return "root" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect("compile");

    assert!(
        build
            .dirs
            .contains(&root.canonicalize().expect("canonicalize")),
        "entity directory missing from {:?}",
        build.dirs
    );
}

/// Auto location and `.file()` paths are emitted relative to the root dir.
#[test]
fn compile_root_auto_location_and_files() {
    let tmp = tempdir().expect("tempdir");
    let album = tmp.path().join("album");
    fs::create_dir_all(&album).expect("album");
    fs::write(album.join("album.toml"), b"[album]").expect("manifest");
    fs::write(album.join("01.flac"), b"audio").expect("track");
    fs::write(album.join("01.flac.lrc"), b"lyrics").expect("lyrics");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("track", {
            kind = "file",
            select = function(parent)
                return d.fs.files(parent.path, { ext = { "flac" } })
            end,
        })
        d.compile.track.file("lyrics", function(ctx) return ctx.path .. ".lrc" end)
        d.compile.track.id(function(ctx) return "track-" .. d.fs.basename(ctx.path) end)

        d.media("album", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { tracks = "track" },
        })
        d.compile.album.file("manifest", function(ctx)
            return d.fs.joinpath(ctx.path, "album.toml")
        end)
        d.compile.album.id(function() return "album" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&album, "album", &album), tmp.path())
        .expect("compile");

    let entity = &build.lock_json["album"];
    assert_eq!(entity["location"]["kind"], "dir");
    assert_eq!(entity["location"]["path"], ".");
    assert_eq!(entity["files"]["manifest"]["path"], "album.toml");
    assert_eq!(entity["tracks"][0]["location"]["kind"], "file");
    assert_eq!(entity["tracks"][0]["location"]["path"], "01.flac");
    assert_eq!(
        entity["tracks"][0]["files"]["lyrics"]["path"],
        "01.flac.lrc"
    );
    assert!(
        build
            .files
            .contains(&album.join("01.flac.lrc").canonicalize().expect("canon"))
    );
}

/// A file parent may own virtual children (M4B chapters).
#[test]
fn compile_root_file_parent_virtual_children() {
    let tmp = tempdir().expect("tempdir");
    let book = tmp.path().join("book");
    fs::create_dir_all(&book).expect("book");
    fs::write(book.join("book.m4b"), b"audio").expect("m4b");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("chapter", {
            kind = "virtual",
            select = function()
                return {
                    { index = 1, title = "One" },
                    { index = 2, title = "Two" },
                }
            end,
        })
        d.compile.chapter.tag("title", function(ctx) return ctx.title end)
        d.compile.chapter.id(function(ctx) return string.format("%03d", ctx.index) end)

        d.media("book_file", {
            kind = "file",
            select = function(parent)
                return d.fs.files(parent.path, { ext = { "m4b" } })
            end,
            children = { chapters = "chapter" },
        })

        d.media("book", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { parts = "book_file" },
        })
        d.compile.book.id(function() return "book" end)
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&book, "book", &book), tmp.path())
        .expect("compile");

    let part = &build.lock_json["book"]["parts"][0];
    assert_eq!(part["location"]["kind"], "file");
    assert_eq!(part["chapters"][0]["tags"]["title"], "One");
    assert!(part["chapters"][0].get("location").is_none());
}

/// Combined kinds dispatch strings to physical kinds and tables to virtual.
#[test]
fn compile_root_combined_kind_dispatch() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("root");
    fs::write(root.join("track.flac"), b"audio").expect("track");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("track", {
            kind = { "file", "virtual" },
            select = function(parent)
                return {
                    parent.path .. "/track.flac",
                    { title = "Virtual" },
                }
            end,
        })
        d.compile.track.id(function(ctx)
            if ctx.path then return "file-track" end
            return "virtual-" .. ctx.title
        end)
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { tracks = "track" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let build = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect("compile");
    let tracks = build.lock_json["root"]["tracks"]
        .as_array()
        .expect("tracks");
    assert_eq!(tracks[0]["location"]["kind"], "file");
    assert_eq!(tracks[0]["id"], "file-track");
    assert!(tracks[1].get("location").is_none());
    assert_eq!(tracks[1]["id"], "virtual-Virtual");
}

/// Virtual-only types reject physical path targets.
#[test]
fn compile_root_virtual_rejects_path() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("root");
    fs::write(root.join("track.flac"), b"audio").expect("track");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "virtual",
            select = function(parent) return { parent.path .. "/track.flac" } end,
        })
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { leaves = "leaf" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("virtual path");
    assert!(err.to_string().contains("virtual-only"));
}

/// A combined parent that resolves to a file cannot own physical children.
#[test]
fn compile_root_file_instance_rejects_physical_child() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("sub")).expect("sub");
    fs::write(root.join("book.m4b"), b"audio").expect("m4b");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "dir",
            select = function(parent) return { parent.path .. "/sub" } end,
        })
        d.media("book_file", {
            kind = { "dir", "file" },
            select = function(parent) return d.fs.files(parent.path, { ext = { "m4b" } }) end,
            children = { leaves = "leaf" },
        })
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { parts = "book_file" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("physical child of file");
    assert!(err.to_string().contains("only have virtual children"));
}

/// Root selections must be directory path strings inside the declared dir.
#[test]
fn discover_rejects_invalid_root_targets() {
    let tmp = tempdir().expect("tempdir");
    let library = tmp.path().join("library");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&library).expect("library");
    fs::create_dir_all(&outside).expect("outside");
    let file = library.join("film.mkv");
    fs::write(&file, b"film").expect("file");

    let engine = LuaEngine::new().expect("engine");
    let code = format!(
        r#"
        d.media("film", {{
            kind = "root",
            dir = "{library}",
            select = function(dir) return {{ dir }} end,
        }})
    "#,
        library = library.to_string_lossy()
    );
    engine.lua.load(&code).exec().expect("register");

    let err = super::discover_targets(&engine, &file, None).expect_err("file root");
    assert!(err.to_string().contains("not an existing directory"));

    let outside_engine = LuaEngine::new().expect("engine");
    let outside_code = format!(
        r#"
        d.media("film", {{
            kind = "root",
            dir = "{library}",
            select = function() return {{ "{outside}" }} end,
        }})
    "#,
        library = library.to_string_lossy(),
        outside = outside.to_string_lossy()
    );
    outside_engine
        .lua
        .load(&outside_code)
        .exec()
        .expect("register");
    let err = super::discover_targets(&outside_engine, &library, None)
        .expect_err("outside root");
    assert!(err.to_string().contains("outside its declared dir"));

    let table_engine = LuaEngine::new().expect("engine");
    let table_code = format!(
        r#"
        d.media("film", {{
            kind = "root",
            dir = "{library}",
            select = function() return {{{{ path = '{library}' }}}} end,
        }})
    "#,
        library = library.to_string_lossy()
    );
    table_engine.lua.load(&table_code).exec().expect("register");
    let err =
        super::discover_targets(&table_engine, &library, None).expect_err("table root");
    assert!(err.to_string().contains("directory path strings"));
}

/// Two root types with different dirs are discovered independently.
#[test]
fn discover_all_supports_multiple_root_dirs() {
    let tmp = tempdir().expect("tempdir");
    let music = tmp.path().join("music");
    let movies = tmp.path().join("movies");
    let album = music.join("Album");
    let film = movies.join("Film");
    for dir in [&album, &film] {
        fs::create_dir_all(dir).expect("dir");
    }
    fs::write(album.join("album.toml"), b"").expect("marker");
    fs::write(film.join("film.toml"), b"").expect("marker");

    let engine = LuaEngine::new().expect("engine");
    let code = format!(
        r#"
        d.media("album", {{
            kind = "root",
            dir = "{music}",
            select = function(dir) return d.fs.roots(dir, "album.toml") end,
        }})
        d.compile.album.id(function() return "album" end)
        d.media("film", {{
            kind = "root",
            dir = "{movies}",
            select = function(dir) return d.fs.roots(dir, "film.toml") end,
        }})
        d.compile.film.id(function() return "film" end)
    "#,
        music = music.to_string_lossy(),
        movies = movies.to_string_lossy()
    );
    engine.lua.load(&code).exec().expect("register");

    let roots = super::discover_targets_all(&engine).expect("roots");
    let mut names: Vec<_> = roots
        .iter()
        .map(|root| (root.root_type.as_str(), root.target.clone()))
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            ("album", album.canonicalize().expect("canon")),
            ("film", film.canonicalize().expect("canon")),
        ]
    );
}

/// Physical children must stay inside their direct parent.
#[test]
fn compile_root_rejects_child_outside_parent() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    let sibling = tmp.path().join("sibling");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&sibling).expect("sibling");

    let engine = LuaEngine::new().expect("engine");
    let code = format!(
        r#"
        d.media("leaf", {{
            kind = "dir",
            select = function() return {{ "{sibling}" }} end,
        }})
        d.media("root", {{
            kind = "root",
            dir = "/",
            select = function() return {{}} end,
            children = {{ leaves = "leaf" }},
        }})
    "#,
        sibling = sibling.to_string_lossy()
    );
    engine.lua.load(&code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("outside child");
    assert!(err.to_string().contains("outside its parent"));
}

/// Selected paths must match the child's declared physical kind.
#[test]
fn compile_root_rejects_kind_mismatch() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("sub")).expect("sub");
    fs::write(root.join("file.txt"), b"x").expect("file");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "file",
            select = function(parent) return { parent.path .. "/sub" } end,
        })
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { leaves = "leaf" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("kind mismatch");
    assert!(err.to_string().contains("does not declare kind 'dir'"));
}

/// Virtual seeds may not overwrite engine-owned fields.
#[test]
fn compile_root_rejects_reserved_virtual_keys() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("root");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "virtual",
            select = function() return { { path = "/tmp/escape" } } end,
        })
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { leaves = "leaf" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("reserved key");
    assert!(err.to_string().contains("reserved key 'path'"));
}

/// Virtual entities have no path, so they must provide an id.
#[test]
fn compile_root_virtual_requires_id() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("root");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("leaf", {
            kind = "virtual",
            select = function() return { { title = "One" } } end,
        })
        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { leaves = "leaf" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("missing id");
    assert!(err.to_string().contains("virtual entity without an id"));
}

/// Two distinct lockfiles may not compile to the same id in one type scope.
#[test]
fn compiler_rejects_duplicate_media_ids_per_type() {
    let mut seen = std::collections::HashMap::new();
    let album_lock = Path::new("/music/Dune/album.lock.json");
    let film_lock = Path::new("/films/Dune/film.lock.json");

    super::super::stream::register_id(&mut seen, "album", "Dune", album_lock)
        .expect("album id");
    super::super::stream::register_id(&mut seen, "film", "Dune", film_lock)
        .expect("film id may reuse album id");

    let other_album = Path::new("/music/Other/album.lock.json");
    let err = super::super::stream::register_id(&mut seen, "album", "Dune", other_album)
        .expect_err("duplicate album id");
    assert!(err.to_string().contains("Duplicate Media ID"));
}

/// Re-emitting the same id for the same lockfile is not a collision.
#[test]
fn compiler_allows_same_lock_duplicate_id() {
    let mut seen = std::collections::HashMap::new();
    let lock = Path::new("/films/Dune/Dune.lock.json");

    super::super::stream::register_id(&mut seen, "film", "Dune", lock).expect("first");
    super::super::stream::register_id(&mut seen, "film", "Dune", lock).expect("second");
}

/// Shallow discovery anchors at the target even when the selector recurses.
#[test]
fn shallow_discovery_ignores_nested_roots() {
    let tmp = tempdir().expect("tempdir");
    let music = tmp.path().join("music");
    let outer = music.join("Outer");
    let inner = outer.join("Inner");
    fs::create_dir_all(&inner).expect("dirs");
    fs::write(outer.join("album.toml"), b"[album]").expect("outer marker");
    fs::write(inner.join("album.toml"), b"[album]").expect("inner marker");

    let engine = LuaEngine::new().expect("engine");
    let code = format!(
        r#"
        d.media("album", {{
            kind = "root",
            dir = "{music}",
            select = function(dir)
                return d.fs.roots(dir, {{ marker = "album.toml", prune = false }})
            end,
        }})
    "#,
        music = music.to_string_lossy()
    );
    engine.lua.load(&code).exec().expect("register");

    let roots = super::discover_targets_shallow(&engine, &outer).expect("shallow");
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].target, outer.canonicalize().expect("canon"));
}

/// Two siblings of one type cannot share an id (Rule 7).
#[test]
fn compile_rejects_duplicate_sibling_ids() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).expect("dir");

    let engine = LuaEngine::new().expect("engine");
    let code = r#"
        d.media("chapter", {
            kind = "virtual",
            select = function() return { {}, {} } end,
        })
        d.compile.chapter.id(function() return "chapter:1" end)

        d.media("root", {
            kind = "root",
            dir = "/",
            select = function() return {} end,
            children = { chapters = "chapter" },
        })
    "#;
    engine.lua.load(code).exec().expect("register");

    let err = compile_root(&engine, &root_target(&root, "root", &root), tmp.path())
        .expect_err("duplicate sibling ids");

    assert!(err.to_string().contains("duplicate sibling child id"));
}
