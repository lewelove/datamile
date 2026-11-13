use crate::lua::LuaEngine;
use mlua::Table;
use std::path::PathBuf;

struct TempFile(PathBuf);

impl TempFile {
    fn new(content: &str) -> Self {
        Self::with_extension(content, "tmp")
    }

    fn with_extension(content: &str, ext: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let name = format!("datamile_test_{}_{}.{ext}", std::process::id(), nanos);
        path.push(name);
        std::fs::write(&path, content).expect("Failed to write temp test file");
        Self(path)
    }

    fn path_str(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let mut path = std::env::temp_dir();
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let name = format!("datamile_test_dir_{}_{}", std::process::id(), nanos);
        path.push(name);
        std::fs::create_dir_all(&path).expect("Failed to create temp test dir");
        Self(path)
    }

    fn path_str(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Verify that `d.fs.basename` extracts the last path component across paths, slashes, and empty inputs.
#[test]
fn test_d_fs_basename() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let file_res: String = engine
        .lua
        .load("return d.fs.basename('/path/to/file.txt')")
        .eval()
        .expect("Execution failed");
    assert_eq!(file_res, "file.txt");

    let dir_res: String = engine
        .lua
        .load("return d.fs.basename('/path/to/directory/')")
        .eval()
        .expect("Execution failed");
    assert_eq!(dir_res, "");

    let multi_slash_res: String = engine
        .lua
        .load("return d.fs.basename('/path/to/nested///')")
        .eval()
        .expect("Execution failed");
    assert_eq!(multi_slash_res, "");

    let root_res: String = engine
        .lua
        .load("return d.fs.basename('/')")
        .eval()
        .expect("Execution failed");
    assert_eq!(root_res, "");

    let nil_res: Option<String> = engine
        .lua
        .load("return d.fs.basename(nil)")
        .eval()
        .expect("Execution failed");
    assert!(nil_res.is_none());

    let empty_res: String = engine
        .lua
        .load("return d.fs.basename('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_res, "");

    let hidden_res: String = engine
        .lua
        .load("return d.fs.basename('/path/to/.hidden')")
        .eval()
        .expect("Execution failed");
    assert_eq!(hidden_res, ".hidden");
}

/// Verify that `d.fs.dirname` extracts the directory path component and handles root boundaries.
#[test]
fn test_d_fs_dirname() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let dir_res: String = engine
        .lua
        .load("return d.fs.dirname('/path/to/file.txt')")
        .eval()
        .expect("Execution failed");
    assert_eq!(dir_res, "/path/to");

    let trailing_res: String = engine
        .lua
        .load("return d.fs.dirname('/path/to/directory/')")
        .eval()
        .expect("Execution failed");
    assert_eq!(trailing_res, "/path/to/directory");

    let root_elem_res: String = engine
        .lua
        .load("return d.fs.dirname('/file.txt')")
        .eval()
        .expect("Execution failed");
    assert_eq!(root_elem_res, "/");

    let root_res: String = engine
        .lua
        .load("return d.fs.dirname('/')")
        .eval()
        .expect("Execution failed");
    assert_eq!(root_res, "/");

    let multi_root_res: String = engine
        .lua
        .load("return d.fs.dirname('///')")
        .eval()
        .expect("Execution failed");
    assert_eq!(multi_root_res, "/");

    let rel_single: String = engine
        .lua
        .load("return d.fs.dirname('file.txt')")
        .eval()
        .expect("Execution failed");
    assert_eq!(rel_single, ".");

    let nil_res: Option<String> = engine
        .lua
        .load("return d.fs.dirname(nil)")
        .eval()
        .expect("Execution failed");
    assert!(nil_res.is_none());

    let empty_res: String = engine
        .lua
        .load("return d.fs.dirname('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_res, ".");
}

/// Verify that `d.fs.joinpath` joins path segments, removes extra separators, and ignores empty items.
#[test]
fn test_d_fs_joinpath() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let joined: String = engine
        .lua
        .load("return d.fs.joinpath('/music', 'Artist - Title [FLAC]', 'cover.jpg')")
        .eval()
        .expect("Execution failed");
    assert_eq!(joined, "/music/Artist - Title [FLAC]/cover.jpg");

    let redundant: String = engine
        .lua
        .load("return d.fs.joinpath('/music/', '//Artist/', '/01.flac')")
        .eval()
        .expect("Execution failed");
    assert_eq!(redundant, "/music/Artist/01.flac");

    let with_empty: String = engine
        .lua
        .load("return d.fs.joinpath('/music', '', 'Artist', nil, '02.flac')")
        .eval()
        .expect("Execution failed");
    assert_eq!(with_empty, "/music/Artist/02.flac");

    let unc_path: String = engine
        .lua
        .load("return d.fs.joinpath('//server', 'share', 'music')")
        .eval()
        .expect("Execution failed");
    assert_eq!(unc_path, "//server/share/music");

    let all_empty: String = engine
        .lua
        .load("return d.fs.joinpath('', nil, '')")
        .eval()
        .expect("Execution failed");
    assert_eq!(all_empty, "");
}

/// Verify that `d.fs.normalize` resolves relative path segments and root boundaries.
#[test]
fn test_d_fs_normalize() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let resolved: String = engine
        .lua
        .load("return d.fs.normalize('/a/b/../c/./d')")
        .eval()
        .expect("Execution failed");
    assert_eq!(resolved, "/a/c/d");

    let root_boundary: String = engine
        .lua
        .load("return d.fs.normalize('/../../a')")
        .eval()
        .expect("Execution failed");
    assert_eq!(root_boundary, "/a");

    let unc_res: String = engine
        .lua
        .load("return d.fs.normalize('//server/share/folder/../file.txt')")
        .eval()
        .expect("Execution failed");
    assert_eq!(unc_res, "//server/share/file.txt");

    let no_expand_res: String = engine
        .lua
        .load("return d.fs.normalize('~/dir', { expand_env = false })")
        .eval()
        .expect("Execution failed");
    assert_eq!(no_expand_res, "~/dir");

    let empty_res: String = engine
        .lua
        .load("return d.fs.normalize('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_res, "");

    let root_res: String = engine
        .lua
        .load("return d.fs.normalize('/')")
        .eval()
        .expect("Execution failed");
    assert_eq!(root_res, "/");
}

/// Verify that `d.fs.parents` iterates upward through all parent directories to root.
#[test]
fn test_d_fs_parents() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let code = r#"
        local parents = {}
        for p in d.fs.parents('/a/b/c/file.txt') do
            table.insert(parents, p)
        end
        return parents
    "#;
    let parents: Vec<String> = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(parents, vec!["/a/b/c", "/a/b", "/a", "/"]);

    let rel_code = r#"
        local parents = {}
        for p in d.fs.parents('a/b/c/file.txt') do
            table.insert(parents, p)
        end
        return parents
    "#;
    let rel_parents: Vec<String> =
        engine.lua.load(rel_code).eval().expect("Execution failed");
    assert_eq!(rel_parents, vec!["a/b/c", "a/b", "a"]);

    let empty_code = r#"
        local count = 0
        for _ in d.fs.parents('/') do
            count = count + 1
        end
        return count
    "#;
    let count: u32 = engine
        .lua
        .load(empty_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(count, 0);
}

/// Verify that `d.fs.dir` lists directory items with specified depth limits and custom skip callbacks.
#[test]
fn test_d_fs_dir() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_dir = TempDir::new();
    let root_path = temp_dir.path_str();

    let sub_a = temp_dir.0.join("sub_a");
    let sub_b = temp_dir.0.join("sub_b");
    let sub_nested = sub_a.join("nested");
    std::fs::create_dir_all(&sub_nested).expect("Failed to create nested dir");
    std::fs::create_dir_all(&sub_b).expect("Failed to create sub_b dir");

    std::fs::write(temp_dir.0.join("root_file.txt"), "root")
        .expect("Failed to write file");
    std::fs::write(sub_a.join("a_file.lrc"), "lyrics").expect("Failed to write file");
    std::fs::write(sub_nested.join("deep.flac"), "audio").expect("Failed to write file");

    let code_depth1 = format!(
        r#"
        local names = {{}}
        for name, type_ in d.fs.dir('{root_path}', {{ depth = 1 }}) do
            names[name] = type_
        end
        return names
    "#
    );
    let names_depth1: Table = engine
        .lua
        .load(&code_depth1)
        .eval()
        .expect("Execution failed");
    assert_eq!(names_depth1.get::<String>("root_file.txt").unwrap(), "file");
    assert_eq!(names_depth1.get::<String>("sub_a").unwrap(), "directory");
    assert_eq!(names_depth1.get::<String>("sub_b").unwrap(), "directory");
    assert!(
        names_depth1
            .get::<Option<String>>("a_file.lrc")
            .unwrap()
            .is_none()
    );

    let code_recursive = format!(
        r#"
        local rel_paths = {{}}
        for rel, type_ in d.fs.dir('{root_path}', {{ depth = 3 }}) do
            rel_paths[rel] = type_
        end
        return rel_paths
    "#
    );
    let rel_paths: Table = engine
        .lua
        .load(&code_recursive)
        .eval()
        .expect("Execution failed");
    assert_eq!(rel_paths.get::<String>("root_file.txt").unwrap(), "file");
    assert_eq!(rel_paths.get::<String>("sub_a/a_file.lrc").unwrap(), "file");
    assert_eq!(
        rel_paths.get::<String>("sub_a/nested/deep.flac").unwrap(),
        "file"
    );

    let code_skip = format!(
        r#"
        local visited = {{}}
        for rel, _ in d.fs.dir('{root_path}', {{
            depth = 3,
            skip = function(dir_rel)
                return dir_rel ~= 'sub_a'
            end
        }}) do
            table.insert(visited, rel)
        end
        return visited
    "#
    );
    let visited: Vec<String> = engine
        .lua
        .load(&code_skip)
        .eval()
        .expect("Execution failed");
    assert!(visited.iter().any(|p| p == "sub_a"));
    assert!(!visited.iter().any(|p| p.starts_with("sub_a/")));
    assert!(visited.iter().any(|p| p == "root_file.txt"));

    let code_skip_calls = format!(
        r#"
        local skip_calls = {{}}
        for rel, _ in d.fs.dir('{root_path}', {{
            depth = 3,
            skip = function(dir_rel)
                table.insert(skip_calls, dir_rel)
                return true
            end
        }}) do end
        return skip_calls
    "#
    );
    let skip_calls: Vec<String> = engine
        .lua
        .load(&code_skip_calls)
        .eval()
        .expect("Execution failed");
    assert!(skip_calls.contains(&"sub_a".to_string()));
    assert!(skip_calls.contains(&"sub_b".to_string()));
    assert_eq!(skip_calls.iter().filter(|p| *p == "sub_a").count(), 1);

    let non_existent_code = r#"
        local count = 0
        for _ in d.fs.dir('/non_existent_datamile_dir_xyz_123') do
            count = count + 1
        end
        return count
    "#;
    let non_existent_count: u32 = engine
        .lua
        .load(non_existent_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(non_existent_count, 0);
}

/// Verify that `d.fs.find` finds files with exact names, name arrays, and downward or upward match functions.
#[test]
fn test_d_fs_find() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_dir = TempDir::new();
    let root_path = temp_dir.path_str();

    let nested_dir = temp_dir.0.join("deep").join("nested");
    std::fs::create_dir_all(&nested_dir).expect("Failed to create nested dir");
    let target_file = nested_dir.join("track.lrc");
    std::fs::write(&target_file, "lyrics content").expect("Failed to write test file");
    let target_file_str = target_file.to_string_lossy().to_string();

    let code_exact = format!(
        r#"return d.fs.find('track.lrc', {{ path = '{root_path}', type = 'file', limit = math.huge }})"#
    );
    let exact_res: Vec<String> = engine
        .lua
        .load(&code_exact)
        .eval()
        .expect("Execution failed");
    assert_eq!(exact_res.len(), 1);
    assert_eq!(exact_res[0], target_file_str);

    let code_table = format!(
        r#"return d.fs.find({{ 'track.lrc', 'nonexistent.txt' }}, {{ path = '{root_path}', type = 'file', limit = math.huge }})"#
    );
    let table_res: Vec<String> = engine
        .lua
        .load(&code_table)
        .eval()
        .expect("Execution failed");
    assert_eq!(table_res.len(), 1);
    assert_eq!(table_res[0], target_file_str);

    let nested_str = nested_dir.to_string_lossy().to_string();
    let code_pred_up = format!(
        r#"
        return d.fs.find(function(name, path)
            return name == 'track.lrc' and path:find('track%.lrc$') ~= nil
        end, {{ path = '{nested_str}', upward = true, limit = 1 }})
    "#
    );
    let pred_up_res: Vec<String> = engine
        .lua
        .load(&code_pred_up)
        .eval()
        .expect("Execution failed");
    assert_eq!(pred_up_res.len(), 1);
    assert_eq!(pred_up_res[0], target_file_str);

    let code_invalid = "return d.fs.find(12345)";
    let invalid_res = engine.lua.load(code_invalid).eval::<Vec<String>>();
    assert!(invalid_res.is_err());
}

/// Verify that `d.fs.root` finds the root directory by searching upward for a marker file and returns nil if missing.
#[test]
fn test_d_fs_root() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_dir = TempDir::new();
    let root_path = temp_dir.path_str();

    let marker_file = temp_dir.0.join("project_marker.toml");
    std::fs::write(&marker_file, "marker = true").expect("Failed to write marker file");

    let deep_dir = temp_dir.0.join("src").join("modules").join("album");
    std::fs::create_dir_all(&deep_dir).expect("Failed to create deep dir");
    let deep_str = deep_dir.to_string_lossy().to_string();

    let code = format!(r#"return d.fs.root('{deep_str}', 'project_marker.toml')"#);
    let root_dir: Option<String> =
        engine.lua.load(&code).eval().expect("Execution failed");
    assert!(root_dir.is_some());
    assert_eq!(root_dir.unwrap(), root_path);

    let missing_code =
        format!(r#"return d.fs.root('{deep_str}', 'non_existent_marker.toml')"#);
    let missing_root: Option<String> = engine
        .lua
        .load(&missing_code)
        .eval()
        .expect("Execution failed");
    assert!(missing_root.is_none());
}

/// Verify that `d.fs.exists` checks if a file exists on disk and returns false for missing or empty paths.
#[test]
fn test_d_fs_exists() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_file = TempFile::new("sample");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.exists('{path_str}')");
    let exists: bool = engine.lua.load(&code).eval().expect("Execution failed");
    assert!(exists);

    let not_exists: bool = engine
        .lua
        .load("return d.fs.exists('/tmp/non_existent_datamile_test_file.tmp')")
        .eval()
        .expect("Execution failed");
    assert!(!not_exists);

    let nil_arg: bool = engine
        .lua
        .load("return d.fs.exists(nil)")
        .eval()
        .expect("Execution failed");
    assert!(!nil_arg);

    let empty_arg: bool = engine
        .lua
        .load("return d.fs.exists('')")
        .eval()
        .expect("Execution failed");
    assert!(!empty_arg);
}

/// Verify that `d.fs.read` reads full file contents, records file dependencies, and returns nil for missing files.
#[test]
fn test_d_fs_read() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_file = TempFile::new("test content");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read('{path_str}')");
    let content: String = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(content, "test content");

    let deps = engine
        .lua
        .app_data_ref::<crate::lua::EngineContext>()
        .unwrap()
        .take_dependencies();
    assert!(deps.iter().any(|d| d.to_string_lossy().contains(&path_str)));

    let missing_code = "return d.fs.read('/tmp/non_existent_datamile_test_file.tmp')";
    let missing_res: Option<String> = engine
        .lua
        .load(missing_code)
        .eval()
        .expect("Execution failed");
    assert!(missing_res.is_none());

    let nil_res: Option<String> = engine
        .lua
        .load("return d.fs.read(nil)")
        .eval()
        .expect("Execution failed");
    assert!(nil_res.is_none());
}

/// Verify that `d.fs.read_lines` reads non-comment lines into indexed and keyed tables.
#[test]
fn test_d_fs_read_lines() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_file = TempFile::new("first\n# comment\n\n   \nsecond\n");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read_lines('{path_str}')");
    let lines: Vec<String> = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "first");
    assert_eq!(lines[1], "second");

    let lookup_code = format!(
        r#"
        local t = d.fs.read_lines('{path_str}')
        return {{ t[1], t['first'], t[2], t['second'] }}
    "#
    );
    let lookup: Vec<mlua::Value> = engine
        .lua
        .load(&lookup_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(lookup.len(), 4);

    let comments_only_file = TempFile::new("# first comment\n# second comment\n");
    let comments_path = comments_only_file.path_str();
    let comments_code = format!("return d.fs.read_lines('{comments_path}')");
    let comments_table: Table = engine
        .lua
        .load(&comments_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(comments_table.raw_len(), 0);

    let missing_code =
        "return d.fs.read_lines('/tmp/non_existent_datamile_test_file.tmp')";
    let missing_lines: Table = engine
        .lua
        .load(missing_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(missing_lines.raw_len(), 0);
}

/// Verify that `d.fs.read_dotenv` parses dotenv files into key-value tables and returns nil if missing.
#[test]
fn test_d_fs_read_dotenv() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let env_content =
        "API_KEY=\"secret_123\"\nPORT=8080\n# comment\nMULTILINE=\"first\nsecond\"\n";
    let temp_file = TempFile::with_extension(env_content, "env");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read_dotenv('{path_str}')");
    let table: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let api_key: String = table.get("API_KEY").expect("Missing API_KEY");
    let port: String = table.get("PORT").expect("Missing PORT");
    let multiline: String = table.get("MULTILINE").expect("Missing MULTILINE");
    assert_eq!(api_key, "secret_123");
    assert_eq!(port, "8080");
    assert_eq!(multiline, "first\nsecond");

    let deps = engine
        .lua
        .app_data_ref::<crate::lua::EngineContext>()
        .unwrap()
        .take_dependencies();
    assert!(deps.iter().any(|d| d.to_string_lossy().contains(&path_str)));

    let missing_code =
        "return d.fs.read_dotenv('/tmp/non_existent_datamile_test_file.env')";
    let missing_res: Option<Table> = engine
        .lua
        .load(missing_code)
        .eval()
        .expect("Execution failed");
    assert!(missing_res.is_none());

    let bad_file = TempFile::with_extension("INVALID LINE WITHOUT EQUALS", "env");
    let bad_path = bad_file.path_str();
    let bad_code = format!("return d.fs.read_dotenv('{bad_path}')");
    let bad_res = engine.lua.load(&bad_code).eval::<Table>();
    assert!(bad_res.is_err());
}

/// Verify that `d.fs.read_json` parses JSON files, enforces json extension, and reports parse errors.
#[test]
fn test_d_fs_read_json() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let json_content = r#"{"artist":"Sample","count":12}"#;
    let temp_file = TempFile::with_extension(json_content, "json");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read_json('{path_str}')");
    let table: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let artist: String = table.get("artist").unwrap();
    let count: u32 = table.get("count").unwrap();
    assert_eq!(artist, "Sample");
    assert_eq!(count, 12);

    let missing_code =
        "return d.fs.read_json('/tmp/non_existent_datamile_test_file.json')";
    let missing_res: Option<Table> = engine
        .lua
        .load(missing_code)
        .eval()
        .expect("Execution failed");
    assert!(missing_res.is_none());

    let bad_file = TempFile::with_extension("{invalid_json", "json");
    let bad_path = bad_file.path_str();
    let bad_code = format!("return d.fs.read_json('{bad_path}')");
    let bad_res = engine.lua.load(&bad_code).eval::<Table>();
    assert!(bad_res.is_err());

    let wrong_ext_file = TempFile::with_extension(json_content, "txt");
    let wrong_ext_path = wrong_ext_file.path_str();
    let wrong_ext_code = format!("return d.fs.read_json('{wrong_ext_path}')");
    let wrong_ext_res = engine.lua.load(&wrong_ext_code).eval::<Table>();
    assert!(wrong_ext_res.is_err());
}

/// Verify that `d.fs.read_toml` parses TOML files, enforces toml extension, and reports parse errors.
#[test]
fn test_d_fs_read_toml() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let toml_content = r#"[album]
artist = "Sample"
year = 2024"#;
    let temp_file = TempFile::with_extension(toml_content, "toml");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read_toml('{path_str}')");
    let table: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let album: Table = table.get("album").unwrap();
    let artist: String = album.get("artist").unwrap();
    let year: u32 = album.get("year").unwrap();
    assert_eq!(artist, "Sample");
    assert_eq!(year, 2024);

    let missing_code =
        "return d.fs.read_toml('/tmp/non_existent_datamile_test_file.toml')";
    let missing_res: Option<Table> = engine
        .lua
        .load(missing_code)
        .eval()
        .expect("Execution failed");
    assert!(missing_res.is_none());

    let bad_file = TempFile::with_extension("[album", "toml");
    let bad_path = bad_file.path_str();
    let bad_code = format!("return d.fs.read_toml('{bad_path}')");
    let bad_res = engine.lua.load(&bad_code).eval::<Table>();
    assert!(bad_res.is_err());

    let wrong_ext_file = TempFile::with_extension(toml_content, "txt");
    let wrong_ext_path = wrong_ext_file.path_str();
    let wrong_ext_code = format!("return d.fs.read_toml('{wrong_ext_path}')");
    let wrong_ext_res = engine.lua.load(&wrong_ext_code).eval::<Table>();
    assert!(wrong_ext_res.is_err());
}

/// Verify that `d.fs.audio_extensions` exposes the Rust supported extension list.
#[test]
fn test_d_fs_audio_extensions() {
    let engine = LuaEngine::new().expect("engine");
    let exts: Table = engine
        .lua
        .load("return d.fs.audio_extensions()")
        .eval()
        .expect("audio extensions");

    let names: Vec<String> = exts
        .sequence_values::<String>()
        .filter_map(Result::ok)
        .collect();

    let expected: Vec<String> = crate::harvest::SUPPORTED_AUDIO_EXTENSIONS
        .iter()
        .map(ToString::to_string)
        .collect();

    assert_eq!(names, expected);
}

fn open_fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").expect("proc fd").count()
}

/// Verify that a recursive scan releases directory handles instead of holding
/// one open file descriptor per visited directory until garbage collection.
#[test]
fn test_d_fs_find_releases_dir_handles() {
    let root = TempDir::new();
    let base = std::path::PathBuf::from(root.path_str());

    for i in 0..200 {
        std::fs::create_dir_all(base.join(format!("album_{i}"))).expect("subdir");
    }

    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = format!(
        "return #d.fs.find('album.toml', {{ path = '{}', type = 'file' }})",
        root.path_str()
    );

    let fd_before = open_fd_count();
    let found: usize = engine.lua.load(&code).eval().expect("Execution failed");
    let fd_after = open_fd_count();

    assert_eq!(found, 0);
    assert!(
        fd_after <= fd_before + 8,
        "scan leaked {} file descriptors",
        fd_after.saturating_sub(fd_before)
    );
}

/// Verify pure path extension/stem helpers and audio extension matching.
#[test]
fn test_d_fs_ext_stem_and_is_audio() {
    let engine = LuaEngine::new().expect("engine");

    let code = r#"
        return {
            ext = d.fs.ext('a/b.FLAC'),
            hidden = d.fs.ext('.hidden'),
            noext = d.fs.ext('noext'),
            stem = d.fs.stem('a/01 - Xtal.flac'),
            hidden_stem = d.fs.stem('.hidden'),
            flac = d.fs.is_audio('song.FLAC'),
            custom = d.fs.is_audio('song.weird', { 'weird' }),
            wrong = d.fs.is_audio('song.weird'),
            no_ext = d.fs.is_audio('song'),
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("fs helpers");

    assert_eq!(res.get::<String>("ext").unwrap(), "flac");
    assert!(res.get::<Option<String>>("hidden").unwrap().is_none());
    assert!(res.get::<Option<String>>("noext").unwrap().is_none());
    assert_eq!(res.get::<String>("stem").unwrap(), "01 - Xtal");
    assert_eq!(res.get::<String>("hidden_stem").unwrap(), ".hidden");
    assert!(res.get::<bool>("flac").unwrap());
    assert!(res.get::<bool>("custom").unwrap());
    assert!(!res.get::<bool>("wrong").unwrap());
    assert!(!res.get::<bool>("no_ext").unwrap());
}

/// Verify stat returns nil for missing paths but still records the parent directory.
#[test]
fn test_d_fs_stat_missing_records_parent() {
    let engine = LuaEngine::new().expect("engine");
    let dir = TempDir::new();
    let missing = format!("{}/missing.toml", dir.path_str());

    let code = format!(
        r#"
        local stat = d.fs.stat('{missing}')
        return {{ missing = stat == nil }}
    "#
    );
    let res: Table = engine.lua.load(&code).eval().expect("stat");
    assert!(res.get::<bool>("missing").unwrap());

    let dirs = engine
        .lua
        .app_data_ref::<crate::lua::EngineContext>()
        .unwrap()
        .take_directories();
    assert!(dirs.contains(&dir.0.canonicalize().unwrap()));

    let existing = TempFile::new("data");
    let code = format!(
        r#"
        local stat = d.fs.stat('{}')
        return {{ kind = stat.is_file, size = stat.byte_size }}
    "#,
        existing.path_str()
    );
    let res: Table = engine.lua.load(&code).eval().expect("stat existing");
    assert!(res.get::<bool>("kind").unwrap());
    assert_eq!(res.get::<u64>("size").unwrap(), 4);
}

/// Verify first returns the highest priority existing candidate and records the directory.
#[test]
fn test_d_fs_first_priority_and_dirty_tracking() {
    let engine = LuaEngine::new().expect("engine");
    let dir = TempDir::new();
    let root = dir.path_str();
    std::fs::write(dir.0.join("cover.png"), b"png").expect("cover");

    let code = format!(r#"return d.fs.first('{root}', {{ 'cover.jpg', 'cover.png' }})"#);
    let found: Option<String> = engine.lua.load(&code).eval().expect("first");
    assert_eq!(found.unwrap(), format!("{root}/cover.png"));

    let dirs = engine
        .lua
        .app_data_ref::<crate::lua::EngineContext>()
        .unwrap()
        .take_directories();
    assert!(dirs.contains(&dir.0.canonicalize().unwrap()));

    let missing: Option<String> = engine
        .lua
        .load("return d.fs.first('/tmp/datamile_no_such_dir_xyz', { 'a.json' })")
        .eval()
        .expect("first missing");
    assert!(missing.is_none());
}

/// Verify files filters by extension, recurses, and sorts naturally by default.
#[test]
fn test_d_fs_files_filter_and_natural_sort() {
    let engine = LuaEngine::new().expect("engine");
    let dir = TempDir::new();
    let root = dir.path_str();
    std::fs::create_dir_all(dir.0.join("sub")).expect("sub");
    for name in ["track10.flac", "track2.flac", "note.mp3", "sub/track1.flac"] {
        std::fs::write(dir.0.join(name), b"x").expect("file");
    }

    let code = format!(
        r#"
        local out = {{}}
        for _, path in ipairs(d.fs.files('{root}', {{ ext = 'FLAC' }})) do
            table.insert(out, d.fs.basename(path))
        end
        return out
    "#
    );
    let names: Vec<String> = engine.lua.load(&code).eval().expect("files");
    assert_eq!(names, vec!["track1.flac", "track2.flac", "track10.flac"]);

    let code = format!(
        r#"
        local out = {{}}
        for _, path in ipairs(d.fs.files('{root}', {{ ext = {{ 'flac', 'mp3' }}, depth = 1, sort = 'lex' }})) do
            table.insert(out, d.fs.basename(path))
        end
        return out
    "#
    );
    let names: Vec<String> = engine.lua.load(&code).eval().expect("files lex");
    assert_eq!(names, vec!["note.mp3", "track10.flac", "track2.flac"]);
}

/// Verify roots selects by marker or predicate, honors pruning, and recursive scopes.
#[test]
fn test_roots_marker_pred() {
    let engine = LuaEngine::new().expect("engine");
    let dir = TempDir::new();
    let root = dir.path_str();
    for path in ["A", "A/sub", "B"] {
        std::fs::create_dir_all(dir.0.join(path)).expect("dirs");
    }
    for path in ["A/album.toml", "A/sub/album.toml", "B/album.toml"] {
        std::fs::write(dir.0.join(path), b"").expect("marker");
    }

    let code = format!(
        r#"
        local out = {{}}
        for _, seed in ipairs(d.fs.roots({{ path = '{root}', recursive = true }}, 'album.toml')) do
            table.insert(out, d.fs.relpath('{root}', seed.path))
        end
        return out
    "#
    );
    let names: Vec<String> = engine.lua.load(&code).eval().expect("roots prune");
    assert_eq!(names, vec!["A", "B"]);

    let code = format!(
        r#"
        local out = {{}}
        for _, seed in ipairs(d.fs.roots({{ path = '{root}', recursive = true }}, {{ marker = 'album.toml', prune = false }})) do
            table.insert(out, d.fs.relpath('{root}', seed.path))
        end
        return out
    "#
    );
    let names: Vec<String> = engine.lua.load(&code).eval().expect("roots no prune");
    assert_eq!(names, vec!["A", "A/sub", "B"]);

    let code = format!(
        r#"
        local out = {{}}
        for _, seed in ipairs(d.fs.roots({{ path = '{root}', recursive = true }}, function(path, entries)
            for _, entry in ipairs(entries) do
                if entry.name == 'album.toml' then
                    return {{ label = d.fs.basename(path) }}, 'prune'
                end
            end
            return false
        end)) do
            table.insert(out, seed.label)
        end
        return out
    "#
    );
    let labels: Vec<String> = engine.lua.load(&code).eval().expect("roots predicate");
    assert_eq!(labels, vec!["A", "B"]);

    let code = format!(
        r#"
        local out = {{}}
        for _, seed in ipairs(d.fs.roots({{ path = '{root}/A', recursive = false }}, 'album.toml')) do
            table.insert(out, d.fs.basename(seed.path))
        end
        return out
    "#
    );
    let names: Vec<String> = engine.lua.load(&code).eval().expect("roots shallow");
    assert_eq!(names, vec!["A"]);
}
