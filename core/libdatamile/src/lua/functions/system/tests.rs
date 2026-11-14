use mlua::Table;

use crate::lua::LuaEngine;

/// Verify that `d.system` executes a command synchronously and captures standard output.
#[test]
fn test_d_system_sync() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "echo", "hello" })
        return { code = res.code, stdout = res.stdout:match("^%s*(.-)%s*$"), ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let code_val: i32 = table.get("code").unwrap();
    let stdout_val: String = table.get("stdout").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(code_val, 0);
    assert_eq!(stdout_val, "hello");
    assert!(ok_val);
}

/// Verify that `d.system` executes a shell command string with pipeline support.
#[test]
fn test_d_system_string_shell() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system("echo 'datamile engine' | tr 'a-z' 'A-Z'")
        return { stdout = res.stdout:match("^%s*(.-)%s*$"), ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let stdout_val: String = table.get("stdout").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(stdout_val, "DATAMILE ENGINE");
    assert!(ok_val);
}

/// Verify that `d.system` writes standard input data to a child process.
#[test]
fn test_d_system_stdin() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "cat" }, { stdin = "testing stdin" })
        return res.stdout
    "#;
    let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(stdout_val, "testing stdin");
}

/// Verify that `d.system` converts a table array of strings to newline-separated stdin.
#[test]
fn test_d_system_stdin_table_lines() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "cat" }, { stdin = { "first line", "second line" } })
        return res.stdout
    "#;
    let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(stdout_val, "first line\nsecond line");
}

/// Verify that `d.system` passes literal control word strings through standard input.
#[test]
fn test_d_system_stdin_literal_strings() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res_inherit = d.system({ "cat" }, { stdin = "inherit" })
        local res_null = d.system({ "cat" }, { stdin = "null" })
        return {
            inherit_out = res_inherit.stdout,
            null_out = res_null.stdout
        }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let inherit_out: String = table.get("inherit_out").unwrap();
    let null_out: String = table.get("null_out").unwrap();
    assert_eq!(inherit_out, "inherit");
    assert_eq!(null_out, "null");
}

/// Verify that `d.system` supports stdio configuration modes.
#[test]
fn test_d_system_stdio_modes() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res_null = d.system({ "echo", "hidden" }, { stdio = "null" })
        local res_pipe = d.system({ "echo", "visible" }, { stdout = "pipe", stderr = "pipe" })
        return {
            null_out = res_null.stdout,
            pipe_out = res_pipe.stdout:match("^%s*(.-)%s*$"),
            pipe_ok = res_pipe.ok
        }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let null_out: String = table.get("null_out").unwrap();
    let pipe_out: String = table.get("pipe_out").unwrap();
    let pipe_ok: bool = table.get("pipe_ok").unwrap();
    assert_eq!(null_out, "");
    assert_eq!(pipe_out, "visible");
    assert!(pipe_ok);
}

/// Verify that `d.system.isatty` reports terminal status.
#[test]
fn test_d_system_isatty() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        return {
            stdin_isatty = d.system.isatty(0),
            default_isatty = d.system.isatty(),
            invalid_isatty = d.system.isatty(-1)
        }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let invalid_val: bool = table.get("invalid_isatty").unwrap();
    assert!(!invalid_val);
}

/// Verify that `d.system` handles large standard input and output streams without deadlocks.
#[test]
fn test_d_system_stdin_large_stress() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let input_chunk = "0123456789abcdef".repeat(4096);
    let code = format!(
        r#"
        local payload = string.rep("{input_chunk}", 4)
        local res = d.system({{ "cat" }}, {{ stdin = payload }})
        return #res.stdout
    "#
    );
    let output_len: usize = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(output_len, input_chunk.len() * 4);
}

/// Verify that `d.system` handles large standard output streams generated by a child process.
#[test]
fn test_d_system_large_output_stress() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "head -c 262144 /dev/zero | tr '\\0' 'A'" })
        return { len = #res.stdout, ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let len: usize = table.get("len").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(len, 262_144);
    assert!(ok_val);
}

/// Verify that `d.system` does not hang when a child process exits before consuming all standard input.
#[test]
fn test_d_system_stdin_early_exit_child() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let input_chunk = "0123456789abcdef".repeat(4096);
    let code = format!(
        r#"
        local payload = string.rep("{input_chunk}", 4)
        local res = d.system({{ "true" }}, {{ stdin = payload }})
        return {{ code = res.code, ok = res.ok }}
    "#
    );
    let table: Table = engine.lua.load(&code).eval().expect("Execution failed");
    let code_val: i32 = table.get("code").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(code_val, 0);
    assert!(ok_val);
}

/// Verify that `d.system` separates standard output and standard error streams.
#[test]
fn test_d_system_stdout_stderr_separation() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "echo 'out stream'; echo 'err stream' >&2" })
        return {
            stdout = res.stdout:match("^%s*(.-)%s*$"),
            stderr = res.stderr:match("^%s*(.-)%s*$"),
            ok = res.ok
        }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let stdout_val: String = table.get("stdout").unwrap();
    let stderr_val: String = table.get("stderr").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(stdout_val, "out stream");
    assert_eq!(stderr_val, "err stream");
    assert!(ok_val);
}

/// Verify that `d.system` returns error exit codes and sets ok to false on process failure.
#[test]
fn test_d_system_nonzero_exit() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "exit 42" })
        return { code = res.code, ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let code_val: i32 = table.get("code").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(code_val, 42);
    assert!(!ok_val);
}

/// Verify that `d.system` sets custom environment variables for a child process.
#[test]
fn test_d_system_env() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "echo $TEST_VAR" }, { env = { TEST_VAR = "injected_val" } })
        return res.stdout:match("^%s*(.-)%s*$")
    "#;
    let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(stdout_val, "injected_val");
}

/// Verify that `d.system` clears parent environment variables when clear_env is enabled.
#[test]
fn test_d_system_clear_env() {
    unsafe {
        std::env::set_var("DATAMILE_TEST_INHERITED_VAR", "present");
    }
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "echo -n $DATAMILE_TEST_INHERITED_VAR" }, { clear_env = true })
        return res.stdout
    "#;
    let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(stdout_val, "");
}

/// Verify that `d.system` allows setting explicit environment variables with clear_env.
#[test]
fn test_d_system_clear_env_with_explicit_vars() {
    unsafe {
        std::env::set_var("DATAMILE_UNWANTED_VAR", "unwanted");
    }
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "echo -n $DATAMILE_UNWANTED_VAR:$EXPLICIT_VAR" }, {
            clear_env = true,
            env = { EXPLICIT_VAR = "kept" }
        })
        return res.stdout
    "#;
    let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(stdout_val, ":kept");
}

/// Verify that `d.system` runs commands in the specified working directory.
#[test]
fn test_d_system_cwd() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let temp_dir = std::env::temp_dir();
    let temp_canon = temp_dir.canonicalize().unwrap_or(temp_dir);
    let temp_str = temp_canon.to_string_lossy();
    let code = format!(
        r#"
        local res = d.system({{ "pwd" }}, {{ cwd = "{temp_str}" }})
        return res.stdout:match("^%s*(.-)%s*$")
    "#
    );
    let stdout_val: String = engine.lua.load(&code).eval().expect("Execution failed");
    assert_eq!(stdout_val, temp_str);
}

/// Verify that `d.system` expands home tilde in working directory option.
#[test]
fn test_d_system_cwd_tilde() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    if let Some(home) = dirs::home_dir() {
        let home_canon = home.canonicalize().unwrap_or(home);
        let home_str = home_canon.to_string_lossy();
        let code = r#"
            local res = d.system({ "pwd" }, { cwd = "~" })
            return res.stdout:match("^%s*(.-)%s*$")
        "#;
        let stdout_val: String = engine.lua.load(code).eval().expect("Execution failed");
        assert_eq!(stdout_val, home_str);
    }
}

/// Verify that `d.system` returns an error when the specified working directory does not exist.
#[test]
fn test_d_system_cwd_invalid() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code =
        r#"return d.system({ "pwd" }, { cwd = "/non_existent_datamile_dir_test_xyz" })"#;
    let res = engine.lua.load(code).eval::<Table>();
    assert!(res.is_err());
}

/// Verify that `d.system` suppresses stream output when stdout or stderr are disabled.
#[test]
fn test_d_system_disable_stdout_stderr() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sh", "-c", "echo out; echo err >&2" }, { stdout = false, stderr = false })
        return { stdout = res.stdout, stderr = res.stderr, ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let stdout_val: String = table.get("stdout").unwrap();
    let stderr_val: String = table.get("stderr").unwrap();
    let ok_val: bool = table.get("ok").unwrap();
    assert_eq!(stdout_val, "");
    assert_eq!(stderr_val, "");
    assert!(ok_val);
}

/// Verify that `d.system` stops a process when the execution time exceeds the timeout.
#[test]
fn test_d_system_timeout() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sleep", "1" }, { timeout = 20 })
        return { ok = res.ok, stderr = res.stderr }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let ok: bool = table.get("ok").unwrap();
    let stderr: String = table.get("stderr").unwrap();
    assert!(!ok);
    assert!(stderr.contains("Process timed out"));
}

/// Verify that `d.system` sets code to nil and sets signal when a process terminates from a signal or timeout.
#[test]
fn test_d_system_signal_and_timeout_code() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let timeout_code = r#"
        local res = d.system({ "sleep", "1" }, { timeout = 20 })
        return { code = res.code, signal = res.signal, ok = res.ok }
    "#;
    let timeout_tbl: Table = engine
        .lua
        .load(timeout_code)
        .eval()
        .expect("Execution failed");
    let timeout_code_val: Option<i32> = timeout_tbl.get("code").unwrap();
    let timeout_sig_val: i32 = timeout_tbl.get("signal").unwrap();
    let timeout_ok_val: bool = timeout_tbl.get("ok").unwrap();
    assert!(timeout_code_val.is_none());
    assert_eq!(timeout_sig_val, 9);
    assert!(!timeout_ok_val);

    let signal_code = r#"
        local res = d.system({ "sh", "-c", "kill -9 $$" })
        return { code = res.code, signal = res.signal, ok = res.ok }
    "#;
    let signal_tbl: Table = engine
        .lua
        .load(signal_code)
        .eval()
        .expect("Execution failed");
    let signal_code_val: Option<i32> = signal_tbl.get("code").unwrap();
    let signal_sig_val: i32 = signal_tbl.get("signal").unwrap();
    let signal_ok_val: bool = signal_tbl.get("ok").unwrap();
    assert!(signal_code_val.is_none());
    assert_eq!(signal_sig_val, 9);
    assert!(!signal_ok_val);
}

/// Verify that `d.system` starts a background process when detach is true.
#[test]
fn test_d_system_detach() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local res = d.system({ "sleep", "0.01" }, { detach = true })
        return { pid = res.pid, ok = res.ok }
    "#;
    let table: Table = engine.lua.load(code).eval().expect("Execution failed");
    let pid: u32 = table.get("pid").unwrap();
    let ok: bool = table.get("ok").unwrap();
    assert!(pid > 0);
    assert!(ok);
}

/// Verify that `d.system` writes standard input and closes the pipe for detached processes without deadlocking.
#[test]
fn test_d_system_detach_with_stdin() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let mut out_path = std::env::temp_dir();
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    out_path.push(format!(
        "datamile_detach_stdin_{}_{}.txt",
        std::process::id(),
        nanos
    ));
    let out_str = out_path.to_string_lossy().to_string();

    let code = format!(
        r#"
        local res = d.system({{ "sh", "-c", "cat > '{out_str}'" }}, {{
            stdin = "detached stdin payload",
            detach = true
        }})
        return res.pid
    "#
    );

    let pid: u32 = engine.lua.load(&code).eval().expect("Execution failed");
    assert!(pid > 0);

    let start = std::time::Instant::now();
    let mut finished = false;
    while start.elapsed() < std::time::Duration::from_millis(1500) {
        if let Ok(content) = std::fs::read_to_string(&out_path) {
            if content == "detached stdin payload" {
                finished = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let _ = std::fs::remove_file(&out_path);
    assert!(
        finished,
        "Detached process deadlocked: stdin pipe was never written and closed with EOF"
    );
}

/// Verify that `d.system` returns an error when command arguments are empty or invalid.
#[test]
fn test_d_system_invalid_cmd() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");

    let empty_str_res = engine.lua.load("return d.system('')").eval::<Table>();
    assert!(empty_str_res.is_err());

    let ws_str_res = engine.lua.load("return d.system('   ')").eval::<Table>();
    assert!(ws_str_res.is_err());

    let empty_tbl_res = engine.lua.load("return d.system({})").eval::<Table>();
    assert!(empty_tbl_res.is_err());

    let invalid_type_res = engine.lua.load("return d.system(12345)").eval::<Table>();
    assert!(invalid_type_res.is_err());

    let nested_tbl_res = engine
        .lua
        .load("return d.system({ {'nested'} })")
        .eval::<Table>();
    assert!(nested_tbl_res.is_err());
}

/// Verify that `d.system` returns an error when the requested executable does not exist.
#[test]
fn test_d_system_nonexistent_binary() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"return d.system({ "non_existent_binary_name_datamile_test" })"#;
    let res = engine.lua.load(code).eval::<Table>();
    assert!(res.is_err());
}

/// Verify that `d.system` executes rapid sequential calls reliably under load.
#[test]
fn test_d_system_rapid_sequential_stress() {
    let engine = LuaEngine::new().expect("Failed to create LuaEngine");
    let code = r#"
        local sum = 0
        for i = 1, 20 do
            local res = d.system({ "echo", tostring(i) })
            if res.ok then
                sum = sum + tonumber(res.stdout:match("^%s*(.-)%s*$"))
            end
        end
        return sum
    "#;
    let sum_val: u32 = engine.lua.load(code).eval().expect("Execution failed");
    assert_eq!(sum_val, 210);
}

/// Verify d.system.open rejects empty and non-string targets before launching.
#[test]
fn test_d_system_open_validation() {
    let engine = LuaEngine::new().expect("engine");

    assert!(engine.lua.load("return d.system.open('')").exec().is_err());
    assert!(
        engine
            .lua
            .load("return d.system.open('   ')")
            .exec()
            .is_err()
    );
    assert!(engine.lua.load("return d.system.open(123)").exec().is_err());
}
