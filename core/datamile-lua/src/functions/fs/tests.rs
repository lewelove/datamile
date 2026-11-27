use std::path::PathBuf;

use mlua::Table;

use crate::LuaRuntime;

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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let code = r"
        local parents = {}
        for p in d.fs.parents('/a/b/c/file.txt') do
            table.insert(parents, p)
        end
        return parents
    ";
    let parents: Vec<String> = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(parents, vec!["/a/b/c", "/a/b", "/a", "/"]);

    let rel_code = r"
        local parents = {}
        for p in d.fs.parents('a/b/c/file.txt') do
            table.insert(parents, p)
        end
        return parents
    ";
    let rel_parents: Vec<String> =
        engine.lua.load(rel_code).eval().expect("Execution failed");
    assert_eq!(rel_parents, vec!["a/b/c", "a/b", "a"]);

    let empty_code = r"
        local count = 0
        for _ in d.fs.parents('/') do
            count = count + 1
        end
        return count
    ";
    let count: u32 = engine
        .lua
        .load(empty_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(count, 0);
}

/// Verify that `d.fs.dir` lists directory items with specified depth limits and custom skip callbacks.
#[allow(
    clippy::too_many_lines,
    reason = "one walk covers depth 1, depth 2 and a skip callback over the same tree"
)]
#[test]
fn test_d_fs_dir() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
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
        r"
        local names = {{}}
        for name, type_ in d.fs.dir('{root_path}', {{ depth = 1 }}) do
            names[name] = type_
        end
        return names
    "
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
        r"
        local rel_paths = {{}}
        for rel, type_ in d.fs.dir('{root_path}', {{ depth = 3 }}) do
            rel_paths[rel] = type_
        end
        return rel_paths
    "
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
        r"
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
    "
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
        r"
        local skip_calls = {{}}
        for rel, _ in d.fs.dir('{root_path}', {{
            depth = 3,
            skip = function(dir_rel)
                table.insert(skip_calls, dir_rel)
                return true
            end
        }}) do end
        return skip_calls
    "
    );
    let skip_calls: Vec<String> = engine
        .lua
        .load(&code_skip_calls)
        .eval()
        .expect("Execution failed");
    assert!(skip_calls.contains(&"sub_a".to_string()));
    assert!(skip_calls.contains(&"sub_b".to_string()));
    assert_eq!(skip_calls.iter().filter(|p| *p == "sub_a").count(), 1);

    let non_existent_code = r"
        local count = 0
        for _ in d.fs.dir('/non_existent_datamile_dir_xyz_123') do
            count = count + 1
        end
        return count
    ";
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
    let temp_dir = TempDir::new();
    let root_path = temp_dir.path_str();

    let nested_dir = temp_dir.0.join("deep").join("nested");
    std::fs::create_dir_all(&nested_dir).expect("Failed to create nested dir");
    let target_file = nested_dir.join("track.lrc");
    std::fs::write(&target_file, "lyrics content").expect("Failed to write test file");
    let target_file_str = target_file.to_string_lossy().to_string();

    let code_exact = format!(
        r"return d.fs.find('track.lrc', {{ path = '{root_path}', type = 'file', limit = math.huge }})"
    );
    let exact_res: Vec<String> = engine
        .lua
        .load(&code_exact)
        .eval()
        .expect("Execution failed");
    assert_eq!(exact_res.len(), 1);
    assert_eq!(exact_res[0], target_file_str);

    let code_table = format!(
        r"return d.fs.find({{ 'track.lrc', 'nonexistent.txt' }}, {{ path = '{root_path}', type = 'file', limit = math.huge }})"
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
        r"
        return d.fs.find(function(name, path)
            return name == 'track.lrc' and path:find('track%.lrc$') ~= nil
        end, {{ path = '{nested_str}', upward = true, limit = 1 }})
    "
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
    let temp_dir = TempDir::new();
    let root_path = temp_dir.path_str();

    let marker_file = temp_dir.0.join("project_marker.toml");
    std::fs::write(&marker_file, "marker = true").expect("Failed to write marker file");

    let deep_dir = temp_dir.0.join("src").join("modules").join("album");
    std::fs::create_dir_all(&deep_dir).expect("Failed to create deep dir");
    let deep_str = deep_dir.to_string_lossy().to_string();

    let code = format!(r"return d.fs.root('{deep_str}', 'project_marker.toml')");
    let root_dir: Option<String> =
        engine.lua.load(&code).eval().expect("Execution failed");
    assert!(root_dir.is_some());
    assert_eq!(root_dir.unwrap(), root_path);

    let missing_code =
        format!(r"return d.fs.root('{deep_str}', 'non_existent_marker.toml')");
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
    let temp_file = TempFile::new("test content");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read('{path_str}')");
    let content: String = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(content, "test content");

    let deps = engine
        .lua
        .app_data_ref::<crate::EngineContext>()
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
    let temp_file = TempFile::new("first\n# comment\n\n   \nsecond\n");
    let path_str = temp_file.path_str();

    let code = format!("return d.fs.read_lines('{path_str}')");
    let lines: Vec<String> = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "first");
    assert_eq!(lines[1], "second");

    let lookup_code = format!(
        r"
        local t = d.fs.read_lines('{path_str}')
        return {{ t[1], t['first'], t[2], t['second'] }}
    "
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
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
        .app_data_ref::<crate::EngineContext>()
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
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
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
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

/// Builds `<root>/<rel>` file trees for the discovery helpers.
fn write_tree(root: &std::path::Path, files: &[&str]) {
    for rel in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, rel).expect("write file");
    }
}

/// `d.fs.roots` walks recursively and returns canonical dirs holding a marker.
#[test]
fn test_d_fs_roots_finds_marker_directories() {
    let root = TempDir::new();
    write_tree(
        &root.0,
        &[
            "A/album.toml",
            "B/virtual.toml",
            "C/notes.txt",
            "D/E/album.toml",
        ],
    );

    let engine = LuaRuntime::new().expect("runtime");
    let code = format!(
        "return d.fs.roots('{root}', {{ markers = {{ 'album.toml', 'virtual.toml' }} }})",
        root = root.path_str()
    );
    let found: Table = engine.lua.load(&code).eval().expect("roots");

    assert_eq!(found.raw_len(), 3);
    let first: String = found.get(1).expect("first");
    assert!(first.ends_with("/A"), "unexpected {first}");
}

/// `d.fs.files` is naturally sorted and stat-enriched in one pass.
#[test]
fn test_d_fs_files_returns_stat_entries() {
    let root = TempDir::new();
    write_tree(&root.0, &["2.flac", "10.flac", "cover.jpg"]);

    let engine = LuaRuntime::new().expect("runtime");
    let code = format!(
        "return d.fs.files('{root}', {{ ext = 'flac' }})",
        root = root.path_str()
    );
    let files: Table = engine.lua.load(&code).eval().expect("files");

    assert_eq!(files.raw_len(), 2);
    let first: Table = files.get(1).expect("first entry");
    assert_eq!(first.get::<String>("name").expect("name"), "2.flac");
    assert_eq!(first.get::<String>("stem").expect("stem"), "2");
    assert_eq!(first.get::<String>("ext").expect("ext"), "flac");
    assert!(first.get::<u64>("byte_size").expect("size") > 0);
    let _mtime: u64 = first.get("mtime").expect("mtime");
    assert!(
        first
            .get::<String>("path")
            .expect("path")
            .ends_with("2.flac")
    );
}

/// `d.fs.files` recurses only when asked, and `paths_only` returns strings.
#[test]
fn test_d_fs_files_recursive_and_paths_only() {
    let root = TempDir::new();
    write_tree(&root.0, &["a.flac", "nested/b.flac"]);

    let engine = LuaRuntime::new().expect("runtime");

    let flat: Table = engine
        .lua
        .load(format!(
            "return d.fs.files('{root}', {{ paths_only = true }})",
            root = root.path_str()
        ))
        .eval()
        .expect("flat");
    assert_eq!(flat.raw_len(), 1);

    let deep: Table = engine
        .lua
        .load(format!(
            "return d.fs.files('{root}', {{ recursive = true, paths_only = true }})",
            root = root.path_str()
        ))
        .eval()
        .expect("deep");
    assert_eq!(deep.raw_len(), 2);
}

/// `d.fs.dirs` lists subdirectories, not files.
#[test]
fn test_d_fs_dirs_lists_subdirectories() {
    let root = TempDir::new();
    write_tree(&root.0, &["disc1/a.flac", "disc2/b.flac", "cover.jpg"]);

    let engine = LuaRuntime::new().expect("runtime");
    let dirs: Table = engine
        .lua
        .load(format!(
            "return d.fs.dirs('{root}')",
            root = root.path_str()
        ))
        .eval()
        .expect("dirs");

    assert_eq!(dirs.raw_len(), 2);
    let first: Table = dirs.get(1).expect("entry");
    assert_eq!(first.get::<String>("name").expect("name"), "disc1");
}

/// `d.fs.first` prefers exact names and falls back to a bare extension.
#[test]
fn test_d_fs_first_matches_names_and_extensions() {
    let root = TempDir::new();
    write_tree(&root.0, &["folder.jpg", "cover.jpg"]);

    let engine = LuaRuntime::new().expect("runtime");

    let exact: Option<String> = engine
        .lua
        .load(format!(
            "return d.fs.first('{root}', {{ 'missing.jpg', 'cover.jpg' }})",
            root = root.path_str()
        ))
        .eval()
        .expect("exact");
    assert!(exact.expect("exact hit").ends_with("cover.jpg"));

    let by_ext: Option<String> = engine
        .lua
        .load(format!(
            "return d.fs.first('{root}', '.jpg')",
            root = root.path_str()
        ))
        .eval()
        .expect("extension");
    assert!(by_ext.expect("extension hit").ends_with("cover.jpg"));

    let missing: Option<String> = engine
        .lua
        .load(format!(
            "return d.fs.first('{root}', 'nope.png')",
            root = root.path_str()
        ))
        .eval()
        .expect("missing");
    assert!(missing.is_none());
}

/// While a compile runs, a lock is not an input: every `d.fs` reader refuses a
/// `*.lock.json` path instead of recording a dependency on it.
#[test]
fn compile_guard_blocks_lock_readers() {
    let lock = TempFile::with_extension(r#"{"album":{}}"#, "lock.json");
    let engine = LuaRuntime::new().expect("runtime");
    let guard = engine.compile_guard().expect("guard");

    for reader in [
        "read",
        "read_lines",
        "exists",
        "read_json",
        "read_toml",
        "read_dotenv",
    ] {
        let code = format!("return d.fs.{reader}('{}')", lock.path_str());
        let err = engine
            .lua
            .load(&code)
            .eval::<mlua::Value>()
            .expect_err("a lock must be refused");
        let message = format!("{err}");
        assert!(
            message.contains("a lock is never an input"),
            "d.fs.{reader}: {message}"
        );
    }

    drop(guard);

    let text: String = engine
        .lua
        .load(format!("return d.fs.read('{}')", lock.path_str()))
        .eval()
        .expect("the same path reads once the compile ends");
    assert!(text.contains("album"), "unexpected: {text}");
}

/// `d.fs.files`, `d.fs.first` and `d.fs.dir` hide the locks written beside the
/// sources, so a compile of a shared dir sees audio and artwork only.
#[test]
fn compile_guard_hides_lock_files_from_enumerators() {
    let root = TempDir::new();
    write_tree(&root.0, &["cover.json", "01.flac", "album.lock.json"]);

    let engine = LuaRuntime::new().expect("runtime");
    let guard = engine.compile_guard().expect("guard");

    let files: Vec<String> = engine
        .lua
        .load(format!(
            "return d.fs.files('{root}', {{ paths_only = true }})",
            root = root.path_str()
        ))
        .eval()
        .expect("files");
    assert_eq!(files.len(), 2, "unexpected: {files:?}");
    assert!(files.iter().all(|path| !path.ends_with(".lock.json")));

    let first: String = engine
        .lua
        .load(format!(
            "return d.fs.first('{root}', {{ 'json' }})",
            root = root.path_str()
        ))
        .eval()
        .expect("first");
    assert!(first.ends_with("cover.json"), "unexpected: {first}");

    let listed: Vec<String> = engine
        .lua
        .load(format!(
            "local out = {{}} for name in d.fs.dir('{root}') do table.insert(out, name) end return out",
            root = root.path_str()
        ))
        .eval()
        .expect("dir");
    assert!(
        listed.iter().all(|name| !name.ends_with(".lock.json")),
        "unexpected: {listed:?}"
    );
    drop(guard);
}

/// Guards nest, and the last drop reopens lock reading: a compile that returns
/// early with `?` must not leave the VM guarded.
#[test]
fn compile_guard_ends_when_the_last_guard_drops() {
    let lock = TempFile::with_extension("{}", "lock.json");
    let engine = LuaRuntime::new().expect("runtime");
    let code = format!("return d.fs.read('{}')", lock.path_str());

    let outer = engine.compile_guard().expect("outer");
    let inner = engine.compile_guard().expect("inner");
    drop(inner);
    assert!(
        engine.lua.load(&code).eval::<mlua::Value>().is_err(),
        "an outer guard still blocks"
    );

    drop(outer);
    assert!(
        engine.lua.load(&code).eval::<mlua::Value>().is_ok(),
        "the lock opens once the compile ends"
    );
}
