use mlua::Table;

use crate::LuaRuntime;

/// Verify that `d.str.startswith` and `d.str.endswith` detect prefixes and suffixes.
#[allow(
    clippy::too_many_lines,
    clippy::cognitive_complexity,
    reason = "one table of inputs and expected results is clearer than four tests"
)]
#[test]
fn test_d_str_starts_and_endswith() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let sw_true: bool = engine
        .lua
        .load("return d.str.startswith('The Caretaker', 'The ')")
        .eval()
        .expect("Execution failed");
    assert!(sw_true);

    let sw_exact: bool = engine
        .lua
        .load("return d.str.startswith('Artist', 'Artist')")
        .eval()
        .expect("Execution failed");
    assert!(sw_exact);

    let sw_empty: bool = engine
        .lua
        .load("return d.str.startswith('Artist', '')")
        .eval()
        .expect("Execution failed");
    assert!(sw_empty);

    let sw_both_empty: bool = engine
        .lua
        .load("return d.str.startswith('', '')")
        .eval()
        .expect("Execution failed");
    assert!(sw_both_empty);

    let sw_longer: bool = engine
        .lua
        .load("return d.str.startswith('hi', 'hello')")
        .eval()
        .expect("Execution failed");
    assert!(!sw_longer);

    let sw_false: bool = engine
        .lua
        .load("return d.str.startswith('Brian Eno', 'Aphex')")
        .eval()
        .expect("Execution failed");
    assert!(!sw_false);

    let sw_case: bool = engine
        .lua
        .load("return d.str.startswith('Aphex Twin', 'aphex')")
        .eval()
        .expect("Execution failed");
    assert!(!sw_case);

    let ew_true: bool = engine
        .lua
        .load("return d.str.endswith('track.flac', '.flac')")
        .eval()
        .expect("Execution failed");
    assert!(ew_true);

    let ew_exact: bool = engine
        .lua
        .load("return d.str.endswith('album', 'album')")
        .eval()
        .expect("Execution failed");
    assert!(ew_exact);

    let ew_empty: bool = engine
        .lua
        .load("return d.str.endswith('track.flac', '')")
        .eval()
        .expect("Execution failed");
    assert!(ew_empty);

    let ew_both_empty: bool = engine
        .lua
        .load("return d.str.endswith('', '')")
        .eval()
        .expect("Execution failed");
    assert!(ew_both_empty);

    let ew_longer: bool = engine
        .lua
        .load("return d.str.endswith('a', 'aaa')")
        .eval()
        .expect("Execution failed");
    assert!(!ew_longer);

    let ew_false: bool = engine
        .lua
        .load("return d.str.endswith('track.flac', '.mp3')")
        .eval()
        .expect("Execution failed");
    assert!(!ew_false);

    let ew_case: bool = engine
        .lua
        .load("return d.str.endswith('track.FLAC', '.flac')")
        .eval()
        .expect("Execution failed");
    assert!(!ew_case);

    let sw_invalid_s = engine
        .lua
        .load("return d.str.startswith(123, '1')")
        .eval::<bool>();
    assert!(sw_invalid_s.is_err());

    let sw_invalid_p = engine
        .lua
        .load("return d.str.startswith('test', nil)")
        .eval::<bool>();
    assert!(sw_invalid_p.is_err());

    let ew_invalid_s = engine
        .lua
        .load("return d.str.endswith({}, '.flac')")
        .eval::<bool>();
    assert!(ew_invalid_s.is_err());

    let ew_invalid_suffix = engine
        .lua
        .load("return d.str.endswith('test', 123)")
        .eval::<bool>();
    assert!(ew_invalid_suffix.is_err());
}

/// Verify that `d.str.trim` strips whitespace from both ends.
#[test]
fn test_d_str_trim() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let trimmed: String = engine
        .lua
        .load("return d.str.trim('  \\n\\t Aphex Twin \\t ')")
        .eval()
        .expect("Execution failed");
    assert_eq!(trimmed, "Aphex Twin");

    let leading_only: String = engine
        .lua
        .load("return d.str.trim('   Leading')")
        .eval()
        .expect("Execution failed");
    assert_eq!(leading_only, "Leading");

    let trailing_only: String = engine
        .lua
        .load("return d.str.trim('Trailing   ')")
        .eval()
        .expect("Execution failed");
    assert_eq!(trailing_only, "Trailing");

    let inner_preserved: String = engine
        .lua
        .load("return d.str.trim('  Multiple   Inner   Spaces  ')")
        .eval()
        .expect("Execution failed");
    assert_eq!(inner_preserved, "Multiple   Inner   Spaces");

    let clean: String = engine
        .lua
        .load("return d.str.trim('Already Clean')")
        .eval()
        .expect("Execution failed");
    assert_eq!(clean, "Already Clean");

    let all_ws: String = engine
        .lua
        .load("return d.str.trim('   \\t\\r\\n  ')")
        .eval()
        .expect("Execution failed");
    assert_eq!(all_ws, "");

    let empty: String = engine
        .lua
        .load("return d.str.trim('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty, "");

    let large_ws_code = r#"
        local s = string.rep(" ", 50000) .. "content" .. string.rep(" \t\n ", 10000)
        return d.str.trim(s)
    "#;
    let large_ws_trimmed: String = engine
        .lua
        .load(large_ws_code)
        .eval()
        .expect("Execution failed");
    assert_eq!(large_ws_trimmed, "content");

    let invalid_nil = engine.lua.load("return d.str.trim(nil)").eval::<String>();
    assert!(invalid_nil.is_err());

    let invalid_num = engine.lua.load("return d.str.trim(12345)").eval::<String>();
    assert!(invalid_num.is_err());
}

/// Verify that `d.str.pesc` escapes Lua pattern special characters.
#[test]
fn test_d_str_pesc() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let escaped: String = engine
        .lua
        .load("return d.str.pesc('100% [FLAC] (2024) + * ? ^ $ - .')")
        .eval()
        .expect("Execution failed");
    assert_eq!(escaped, "100%% %[FLAC%] %(2024%) %+ %* %? %^ %$ %- %.");

    let normal: String = engine
        .lua
        .load("return d.str.pesc('normal_text_123')")
        .eval()
        .expect("Execution failed");
    assert_eq!(normal, "normal_text_123");

    let empty: String = engine
        .lua
        .load("return d.str.pesc('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty, "");

    let invalid_type = engine.lua.load("return d.str.pesc(false)").eval::<String>();
    assert!(invalid_type.is_err());
}

/// Verify that `d.str.uri_encode` percent-encodes every byte outside the
/// RFC 3986 unreserved set and leaves that set untouched.
#[test]
fn test_d_str_uri_encode() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let query: String = engine
        .lua
        .load("return d.str.uri_encode('artist:\"Aphex Twin\" AND releasegroup:\"SAW 85-92\"')")
        .eval()
        .expect("Execution failed");
    assert_eq!(
        query,
        "artist%3A%22Aphex%20Twin%22%20AND%20releasegroup%3A%22SAW%2085-92%22"
    );

    // The unreserved set is the one thing that must survive untouched.
    let unreserved: String = engine
        .lua
        .load("return d.str.uri_encode('AZaz09-._~')")
        .eval()
        .expect("Execution failed");
    assert_eq!(unreserved, "AZaz09-._~");

    // Multi-byte UTF-8 becomes one escape per byte, not per code point.
    let multibyte: String = engine
        .lua
        .load("return d.str.uri_encode('‽')")
        .eval()
        .expect("Execution failed");
    assert_eq!(multibyte, "%E2%80%BD");

    let empty: String = engine
        .lua
        .load("return d.str.uri_encode('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty, "");

    let invalid_type = engine
        .lua
        .load("return d.str.uri_encode(nil)")
        .eval::<String>();
    assert!(invalid_type.is_err());
}

/// Verify that `d.str.stricmp` performs case-insensitive ASCII comparison.
#[test]
fn test_d_str_stricmp() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let equal: i32 = engine
        .lua
        .load("return d.str.stricmp('Aphex Twin', 'APHEX TWIN')")
        .eval()
        .expect("Execution failed");
    assert_eq!(equal, 0);

    let equal_empty: i32 = engine
        .lua
        .load("return d.str.stricmp('', '')")
        .eval()
        .expect("Execution failed");
    assert_eq!(equal_empty, 0);

    let less: i32 = engine
        .lua
        .load("return d.str.stricmp('ambient', 'Rock')")
        .eval()
        .expect("Execution failed");
    assert_eq!(less, -1);

    let greater: i32 = engine
        .lua
        .load("return d.str.stricmp('Vaporwave', 'ambient')")
        .eval()
        .expect("Execution failed");
    assert_eq!(greater, 1);

    let empty_vs_val: i32 = engine
        .lua
        .load("return d.str.stricmp('', 'a')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_vs_val, -1);

    let invalid_arg = engine
        .lua
        .load("return d.str.stricmp('test', 123)")
        .eval::<i32>();
    assert!(invalid_arg.is_err());
}

/// Verify that `d.str.split` and `d.str.gsplit` tokenize strings with plain and pattern options.
#[allow(
    clippy::cognitive_complexity,
    reason = "plain and pattern splits are asserted against one shared input table"
)]
#[test]
fn test_d_str_split_and_gsplit() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let basic_split: Table = engine
        .lua
        .load("return d.str.split(':aa::b:', ':')")
        .eval()
        .expect("Execution failed");
    assert_eq!(basic_split.raw_len(), 5);
    assert_eq!(basic_split.get::<String>(1).unwrap(), "");
    assert_eq!(basic_split.get::<String>(2).unwrap(), "aa");
    assert_eq!(basic_split.get::<String>(3).unwrap(), "");
    assert_eq!(basic_split.get::<String>(4).unwrap(), "b");
    assert_eq!(basic_split.get::<String>(5).unwrap(), "");

    let plain_split: Table = engine
        .lua
        .load("return d.str.split('x*yz*o', '*', { plain = true })")
        .eval()
        .expect("Execution failed");
    assert_eq!(plain_split.raw_len(), 3);
    assert_eq!(plain_split.get::<String>(1).unwrap(), "x");
    assert_eq!(plain_split.get::<String>(2).unwrap(), "yz");
    assert_eq!(plain_split.get::<String>(3).unwrap(), "o");

    let plain_bool_shorthand: Table = engine
        .lua
        .load("return d.str.split('a.b.c', '.', true)")
        .eval()
        .expect("Execution failed");
    assert_eq!(plain_bool_shorthand.raw_len(), 3);
    assert_eq!(plain_bool_shorthand.get::<String>(1).unwrap(), "a");
    assert_eq!(plain_bool_shorthand.get::<String>(2).unwrap(), "b");
    assert_eq!(plain_bool_shorthand.get::<String>(3).unwrap(), "c");

    let trimempty_split: Table = engine
        .lua
        .load("return d.str.split('|x|y|z|', '|', { trimempty = true })")
        .eval()
        .expect("Execution failed");
    assert_eq!(trimempty_split.raw_len(), 3);
    assert_eq!(trimempty_split.get::<String>(1).unwrap(), "x");
    assert_eq!(trimempty_split.get::<String>(2).unwrap(), "y");
    assert_eq!(trimempty_split.get::<String>(3).unwrap(), "z");

    let char_split: Table = engine
        .lua
        .load("return d.str.split('abc', '')")
        .eval()
        .expect("Execution failed");
    assert_eq!(char_split.raw_len(), 3);
    assert_eq!(char_split.get::<String>(1).unwrap(), "a");
    assert_eq!(char_split.get::<String>(2).unwrap(), "b");
    assert_eq!(char_split.get::<String>(3).unwrap(), "c");

    let no_match_split: Table = engine
        .lua
        .load("return d.str.split('entire_string', ';')")
        .eval()
        .expect("Execution failed");
    assert_eq!(no_match_split.raw_len(), 1);
    assert_eq!(no_match_split.get::<String>(1).unwrap(), "entire_string");

    let gsplit_iter: String = engine
        .lua
        .load(
            "local out = ''; for s in d.str.gsplit('a,b,c', ',') do out = out .. s end; return out",
        )
        .eval()
        .expect("Execution failed");
    assert_eq!(gsplit_iter, "abc");

    let split_invalid_s = engine
        .lua
        .load("return d.str.split(123, ',')")
        .eval::<Table>();
    assert!(split_invalid_s.is_err());

    let split_invalid_sep = engine
        .lua
        .load("return d.str.split('a,b', nil)")
        .eval::<Table>();
    assert!(split_invalid_sep.is_err());
}

/// Verify that `d.str.word_count` counts whitespace-delimited tokens.
#[test]
fn test_d_str_word_count() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");

    let count: i64 = engine
        .lua
        .load("return d.str.word_count('Selected Ambient Works Volume II')")
        .eval()
        .expect("Execution failed");
    assert_eq!(count, 5);

    let single_word: i64 = engine
        .lua
        .load("return d.str.word_count('Aphex')")
        .eval()
        .expect("Execution failed");
    assert_eq!(single_word, 1);

    let empty_count: i64 = engine
        .lua
        .load("return d.str.word_count('   \\t\\n ')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_count, 0);

    let empty_str: i64 = engine
        .lua
        .load("return d.str.word_count('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_str, 0);

    let invalid_type = engine
        .lua
        .load("return d.str.word_count(123)")
        .eval::<i64>();
    assert!(invalid_type.is_err());
}

/// Verify that `d.str.utf_pos`, `d.str.byteindex`, and `d.str.utfindex` handle multibyte sequences.
#[allow(
    clippy::cognitive_complexity,
    reason = "index and byte positions of one multibyte string are checked together"
)]
#[test]
fn test_d_str_utf_indexing() {
    let engine = LuaRuntime::new().expect("Failed to create LuaRuntime");
    let code = r#"
        local str = "Datamile 🎵 音楽"
        return {
            pos = d.str.utf_pos(str),
            b_idx = d.str.byteindex(str, 6),
            b_first = d.str.byteindex(str, 1),
            b_out = d.str.byteindex(str, 999),
            b_neg = d.str.byteindex(str, -1),
            u_idx = d.str.utfindex(str, 6),
            u_inside = d.str.utfindex(str, 11),
            u_first = d.str.utfindex(str, 1),
            u_end = d.str.utfindex(str),
            u_start = d.str.utf_start(str, 11),
            u_end_pos = d.str.utf_end(str, 10),
            u_ascii_start = d.str.utf_start(str, 1),
            u_ascii_end = d.str.utf_end(str, 1)
        }
    "#;
    let res: Table = engine.lua.load(code).eval().expect("Execution failed");
    let pos: Table = res.get("pos").unwrap();
    assert_eq!(pos.raw_len(), 13);
    assert_eq!(res.get::<i64>("b_idx").unwrap(), 6);
    assert_eq!(res.get::<i64>("b_first").unwrap(), 1);
    assert_eq!(res.get::<i64>("b_out").unwrap(), 21);
    assert_eq!(res.get::<i64>("b_neg").unwrap(), 1);
    assert_eq!(res.get::<i64>("u_idx").unwrap(), 6);
    assert_eq!(res.get::<i64>("u_inside").unwrap(), 10);
    assert_eq!(res.get::<i64>("u_first").unwrap(), 1);
    assert_eq!(res.get::<i64>("u_end").unwrap(), 13);
    assert_eq!(res.get::<i64>("u_start").unwrap(), -1);
    assert_eq!(res.get::<i64>("u_end_pos").unwrap(), 3);
    assert_eq!(res.get::<i64>("u_ascii_start").unwrap(), 0);
    assert_eq!(res.get::<i64>("u_ascii_end").unwrap(), 0);

    let empty_pos: Table = engine
        .lua
        .load("return d.str.utf_pos('')")
        .eval()
        .expect("Execution failed");
    assert_eq!(empty_pos.raw_len(), 0);

    let utf_start_err = engine
        .lua
        .load("return d.str.utf_start('test', 999)")
        .eval::<i64>();
    assert!(utf_start_err.is_err());

    let utf_end_err = engine
        .lua
        .load("return d.str.utf_end('test', 0)")
        .eval::<i64>();
    assert!(utf_end_err.is_err());

    let byteindex_err_s = engine
        .lua
        .load("return d.str.byteindex(123, 1)")
        .eval::<i64>();
    assert!(byteindex_err_s.is_err());

    let byteindex_err_idx = engine
        .lua
        .load("return d.str.byteindex('test', 'not_num')")
        .eval::<i64>();
    assert!(byteindex_err_idx.is_err());
}

/// Only a leading digit run is an integer; `"disc2"` has none.
#[test]
fn test_d_str_leading_int() {
    let engine = LuaRuntime::new().expect("runtime");

    let cases = [
        ("03 - Title.flac", Some(3i64)),
        ("3/12", Some(3)),
        ("disc2", None),
        ("  12abc", Some(12)),
    ];

    for (input, expected) in cases {
        let code = format!("return d.str.leading_int('{input}')");
        let got: Option<i64> = engine.lua.load(&code).eval().expect("leading_int");
        assert_eq!(got, expected, "input {input}");
    }
}
