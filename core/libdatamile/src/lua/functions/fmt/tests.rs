use crate::lua::LuaEngine;

/// Verify compact human durations, including nil and negative inputs.
#[test]
fn test_d_fmt_duration() {
    let engine = LuaEngine::new().expect("engine");

    let cases = [
        ("0", "0m"),
        ("2700000", "45m"),
        ("5400000", "1h 30m"),
        ("3600000", "1h"),
        ("-1", "0m"),
        ("nil", "0m"),
    ];

    for (input, expected) in cases {
        let code = format!("return d.fmt.duration({input})");
        let out: String = engine.lua.load(&code).eval().expect("duration");
        assert_eq!(out, expected, "input {input}");
    }
}

/// Verify clock durations, including nil and negative inputs.
#[test]
fn test_d_fmt_clock() {
    let engine = LuaEngine::new().expect("engine");

    let cases = [
        ("0", "0:00"),
        ("65000", "1:05"),
        ("3665000", "1:01:05"),
        ("-1", "0:00"),
        ("nil", "0:00"),
    ];

    for (input, expected) in cases {
        let code = format!("return d.fmt.clock({input})");
        let out: String = engine.lua.load(&code).eval().expect("clock");
        assert_eq!(out, expected, "input {input}");
    }
}
