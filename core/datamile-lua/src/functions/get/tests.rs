use crate::LuaRuntime;

/// Verify that `d.get` reads nested table values across dot paths, arguments, array indexes, and missing fields.
#[test]
fn test_d_get() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let test_setup = r#"
        local sample = {
            metadata = {
                album = {
                    title = "Selected Ambient Works",
                    artist = "Aphex Twin"
                },
                tracks = {
                    { title = "Xtal", duration = 294 },
                    { title = "Tha", duration = 546 }
                }
            }
        }
    "#;

    let dot_path_res: String = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata.album.title')"
        ))
        .eval()
        .expect("Execution failed");
    assert_eq!(dot_path_res, "Selected Ambient Works");

    let varargs_res: String = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata', 'album', 'artist')"
        ))
        .eval()
        .expect("Execution failed");
    assert_eq!(varargs_res, "Aphex Twin");

    let mixed_res: String = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata.album', 'title')"
        ))
        .eval()
        .expect("Execution failed");
    assert_eq!(mixed_res, "Selected Ambient Works");

    let array_index_res: String = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata.tracks', 2, 'title')"
        ))
        .eval()
        .expect("Execution failed");
    assert_eq!(array_index_res, "Tha");

    let array_dot_res: u32 = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata.tracks.1.duration')"
        ))
        .eval()
        .expect("Execution failed");
    assert_eq!(array_dot_res, 294);

    let missing_path: Option<String> = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'metadata.nonexistent.field')"
        ))
        .eval()
        .expect("Execution failed");
    assert!(missing_path.is_none());

    let nil_target: Option<String> = engine
        .lua
        .load("return d.get(nil, 'metadata.album.title')")
        .eval()
        .expect("Execution failed");
    assert!(nil_target.is_none());
}

/// Verify that `d.get` preserves boolean false values when accessing array indices and nested tables.
#[test]
fn test_d_get_boolean_false() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let test_setup = r"
        local sample = {
            flags = { false, true },
            nested = {
                disabled = false
            }
        }
    ";

    let array_dot_res: bool = engine
        .lua
        .load(format!("{test_setup}\nreturn d.get(sample, 'flags.1')"))
        .eval()
        .expect("Execution failed");
    assert!(!array_dot_res);

    let array_index_res: bool = engine
        .lua
        .load(format!("{test_setup}\nreturn d.get(sample, 'flags', 1)"))
        .eval()
        .expect("Execution failed");
    assert!(!array_index_res);

    let nested_res: bool = engine
        .lua
        .load(format!(
            "{test_setup}\nreturn d.get(sample, 'nested.disabled')"
        ))
        .eval()
        .expect("Execution failed");
    assert!(!nested_res);
}
