use crate::LuaRuntime;

/// Durations render as `M:SS` below an hour and `H:MM:SS` above it.
#[test]
fn test_d_fmt_duration() {
    let engine = LuaRuntime::new().expect("runtime");

    let cases = [
        (0, "0:00"),
        (59_000, "0:59"),
        (180_000, "3:00"),
        (3_600_000, "1:00:00"),
        (3_723_000, "1:02:03"),
    ];

    for (ms, expected) in cases {
        let got: String = engine
            .lua
            .load(format!("return d.fmt.duration({ms})"))
            .eval()
            .expect("duration");
        assert_eq!(got, expected, "ms {ms}");
    }
}

/// Bytes stay whole below a kilobyte and get one decimal above it.
#[test]
fn test_d_fmt_bytes() {
    let engine = LuaRuntime::new().expect("runtime");

    assert_eq!(
        engine
            .lua
            .load("return d.fmt.bytes(512)")
            .eval::<String>()
            .expect("bytes"),
        "512 B"
    );
    assert_eq!(
        engine
            .lua
            .load("return d.fmt.bytes(1572864)")
            .eval::<String>()
            .expect("bytes"),
        "1.5 MB"
    );
}
