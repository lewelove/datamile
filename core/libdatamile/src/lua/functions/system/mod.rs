#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::{IsTerminal, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mlua::{Lua, Table, Value};

type ReaderHandle = JoinHandle<Vec<u8>>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvMode {
    Inherit,
    Clear,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StdioMode {
    Pipe,
    Inherit,
    Null,
}

impl StdioMode {
    fn to_stdio(self) -> Stdio {
        match self {
            Self::Pipe => Stdio::piped(),
            Self::Inherit => Stdio::inherit(),
            Self::Null => Stdio::null(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExecutionMode {
    Sync,
    Detach,
}

struct SystemOpts {
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    clear_env: EnvMode,
    stdin: Option<String>,
    stdin_mode: StdioMode,
    stdout: StdioMode,
    stderr: StdioMode,
    timeout: Option<u64>,
    mode: ExecutionMode,
}

fn is_terminal_fd(fd: i32) -> bool {
    match fd {
        0 => std::io::stdin().is_terminal(),
        1 => std::io::stdout().is_terminal(),
        2 => std::io::stderr().is_terminal(),
        _ => false,
    }
}

fn parse_stdio_value(val: Option<&Value>, default: StdioMode) -> mlua::Result<StdioMode> {
    let Some(v) = val else {
        return Ok(default);
    };

    match v {
        Value::Nil => Ok(default),
        Value::Boolean(true) => Ok(StdioMode::Pipe),
        Value::Boolean(false) => Ok(StdioMode::Null),
        Value::String(s) => match &*s.to_str()? {
            "pipe" => Ok(StdioMode::Pipe),
            "inherit" => Ok(StdioMode::Inherit),
            "null" => Ok(StdioMode::Null),
            other => Err(mlua::Error::runtime(format!(
                "invalid stdio mode '{other}', expected 'pipe', 'inherit', or 'null'"
            ))),
        },
        _ => Err(mlua::Error::runtime("stdio mode must be string or boolean")),
    }
}

fn parse_stdin_option(
    tbl: &Table,
    default_mode: StdioMode,
) -> mlua::Result<(Option<String>, StdioMode)> {
    let mut stdin_str: Option<String> = None;
    let explicit_mode = tbl.get::<Option<Value>>("stdin_mode")?;
    let mut stdin_mode = parse_stdio_value(explicit_mode.as_ref(), default_mode)?;

    let Ok(stdin_val) = tbl.get::<Option<Value>>("stdin") else {
        return Ok((stdin_str, stdin_mode));
    };

    let Some(val) = stdin_val else {
        return Ok((stdin_str, stdin_mode));
    };

    match val {
        Value::String(s) => {
            stdin_str = Some(s.to_str()?.to_string());
            if explicit_mode.is_none() {
                stdin_mode = StdioMode::Pipe;
            }
        }
        Value::Table(lines_tbl) => {
            let mut lines = Vec::new();
            for line in lines_tbl.sequence_values::<String>() {
                lines.push(line?);
            }
            stdin_str = Some(lines.join("\n"));
            if explicit_mode.is_none() {
                stdin_mode = StdioMode::Pipe;
            }
        }
        Value::Boolean(true) if explicit_mode.is_none() => {
            stdin_mode = StdioMode::Pipe;
        }
        Value::Boolean(false) if explicit_mode.is_none() => {
            stdin_mode = StdioMode::Null;
        }
        _ => {}
    }

    Ok((stdin_str, stdin_mode))
}

fn parse_opts(opts_val: Option<Table>) -> mlua::Result<SystemOpts> {
    let Some(tbl) = opts_val else {
        return Ok(SystemOpts {
            cwd: None,
            env: None,
            clear_env: EnvMode::Inherit,
            stdin: None,
            stdin_mode: StdioMode::Null,
            stdout: StdioMode::Pipe,
            stderr: StdioMode::Pipe,
            timeout: None,
            mode: ExecutionMode::Sync,
        });
    };

    let is_detach = tbl.get::<Option<bool>>("detach")?.unwrap_or(false);
    let clear_env = if tbl.get::<Option<bool>>("clear_env")?.unwrap_or(false) {
        EnvMode::Clear
    } else {
        EnvMode::Inherit
    };

    let fallback_stdio = if is_detach {
        StdioMode::Null
    } else {
        StdioMode::Pipe
    };

    let global_val = tbl.get::<Option<Value>>("stdio")?;
    let global_stdio = parse_stdio_value(global_val.as_ref(), fallback_stdio)?;

    let stdout_val = tbl.get::<Option<Value>>("stdout")?;
    let stdout = parse_stdio_value(stdout_val.as_ref(), global_stdio)?;

    let stderr_val = tbl.get::<Option<Value>>("stderr")?;
    let stderr = parse_stdio_value(stderr_val.as_ref(), global_stdio)?;

    let default_stdin_mode = if global_val.is_some() {
        global_stdio
    } else {
        StdioMode::Null
    };

    let (stdin, stdin_mode) = parse_stdin_option(&tbl, default_stdin_mode)?;

    let mode = if is_detach {
        ExecutionMode::Detach
    } else {
        ExecutionMode::Sync
    };

    Ok(SystemOpts {
        cwd: tbl.get("cwd")?,
        env: tbl.get("env")?,
        clear_env,
        stdin,
        stdin_mode,
        stdout,
        stderr,
        timeout: tbl.get("timeout")?,
        mode,
    })
}

fn parse_cmd(cmd_val: Value) -> mlua::Result<Vec<String>> {
    match cmd_val {
        Value::Table(tbl) => {
            let mut args = Vec::new();
            for item in tbl.sequence_values::<String>() {
                args.push(item?);
            }
            if args.is_empty() || args[0].trim().is_empty() {
                return Err(mlua::Error::runtime("cmd table cannot be empty"));
            }
            Ok(args)
        }
        Value::String(s) => {
            let cmd = s.to_str()?.to_string();
            if cmd.trim().is_empty() {
                return Err(mlua::Error::runtime("cmd string cannot be empty"));
            }
            Ok(vec!["sh".to_string(), "-c".to_string(), cmd])
        }
        _ => Err(mlua::Error::runtime(
            "cmd must be a table of strings or a string",
        )),
    }
}

fn build_command(cmd_args: &[String], opts: &SystemOpts) -> Command {
    let mut command = Command::new(&cmd_args[0]);
    if cmd_args.len() > 1 {
        command.args(&cmd_args[1..]);
    }

    if opts.clear_env == EnvMode::Clear {
        command.env_clear();
    }
    if let Some(ref env_map) = opts.env {
        for (k, v) in env_map {
            command.env(k, v);
        }
    }
    if let Some(ref cwd) = opts.cwd {
        command.current_dir(crate::utils::expand_path(cwd));
    }
    command
}

fn spawn_detached(
    mut command: Command,
    opts: &SystemOpts,
    lua: &Lua,
) -> mlua::Result<Value> {
    command.stdin(opts.stdin_mode.to_stdio());
    command.stdout(opts.stdout.to_stdio());
    command.stderr(opts.stderr.to_stdio());
    command.process_group(0);

    let mut child = command.spawn().map_err(mlua::Error::external)?;
    let pid = child.id();

    if let Some(mut child_stdin) = child.stdin.take()
        && let Some(ref stdin_str) = opts.stdin
    {
        let bytes = stdin_str.as_bytes().to_vec();
        thread::spawn(move || {
            let _ = child_stdin.write_all(&bytes);
        });
    }

    thread::spawn(move || {
        let _ = child.wait();
    });

    let ret = lua.create_table()?;
    ret.set("pid", pid)?;
    ret.set("ok", true)?;
    Ok(Value::Table(ret))
}

fn spawn_stream_readers(
    child: &mut Child,
    stdout_mode: StdioMode,
    stderr_mode: StdioMode,
) -> (Option<ReaderHandle>, Option<ReaderHandle>) {
    let stdout_handle = if stdout_mode == StdioMode::Pipe {
        child.stdout.take().map(|mut r| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf);
                buf
            })
        })
    } else {
        None
    };

    let stderr_handle = if stderr_mode == StdioMode::Pipe {
        child.stderr.take().map(|mut r| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf);
                buf
            })
        })
    } else {
        None
    };

    (stdout_handle, stderr_handle)
}

fn build_result_table(
    status: ExitStatus,
    stdout_bytes: &[u8],
    stderr_bytes: &[u8],
    lua: &Lua,
) -> mlua::Result<Value> {
    let code_val = status
        .code()
        .map_or(Value::Nil, |c| Value::Integer(i64::from(c)));
    let signal_val = status
        .signal()
        .map_or(Value::Nil, |s| Value::Integer(i64::from(s)));

    let ret = lua.create_table()?;
    ret.set("code", code_val)?;
    ret.set("signal", signal_val)?;
    ret.set("stdout", String::from_utf8_lossy(stdout_bytes).into_owned())?;
    ret.set("stderr", String::from_utf8_lossy(stderr_bytes).into_owned())?;
    ret.set("ok", status.success())?;

    Ok(Value::Table(ret))
}

fn wait_for_exit(
    mut child: Child,
    timeout_dur: Option<Duration>,
    stdout_handle: Option<ReaderHandle>,
    stderr_handle: Option<ReaderHandle>,
    lua: &Lua,
) -> mlua::Result<Value> {
    let status = match timeout_dur {
        None => child.wait().map_err(mlua::Error::external)?,
        Some(dur) => {
            let start = Instant::now();
            loop {
                if let Some(status) = child.try_wait().map_err(mlua::Error::external)? {
                    break status;
                }
                if start.elapsed() >= dur {
                    let _ = child.kill();
                    let status = child.wait().map_err(mlua::Error::external)?;
                    let stdout_bytes = stdout_handle
                        .map_or_else(Vec::new, |h| h.join().unwrap_or_default());
                    let stderr_bytes = stderr_handle
                        .map_or_else(Vec::new, |h| h.join().unwrap_or_default());
                    let timeout_ms = dur.as_millis();
                    let stderr_msg = String::from_utf8_lossy(&stderr_bytes);

                    let code_val = status
                        .code()
                        .map_or(Value::Nil, |c| Value::Integer(i64::from(c)));
                    let signal_val = status
                        .signal()
                        .map_or(Value::Integer(9), |s| Value::Integer(i64::from(s)));

                    let ret = lua.create_table()?;
                    ret.set("code", code_val)?;
                    ret.set("signal", signal_val)?;
                    ret.set(
                        "stdout",
                        String::from_utf8_lossy(&stdout_bytes).into_owned(),
                    )?;
                    ret.set(
                        "stderr",
                        format!("Process timed out after {timeout_ms}ms\n{stderr_msg}"),
                    )?;
                    ret.set("ok", false)?;
                    return Ok(Value::Table(ret));
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
    };

    let stdout_bytes =
        stdout_handle.map_or_else(Vec::new, |h| h.join().unwrap_or_default());
    let stderr_bytes =
        stderr_handle.map_or_else(Vec::new, |h| h.join().unwrap_or_default());

    build_result_table(status, &stdout_bytes, &stderr_bytes, lua)
}

fn execute_sync(
    mut command: Command,
    opts: SystemOpts,
    lua: &Lua,
) -> mlua::Result<Value> {
    command.stdin(opts.stdin_mode.to_stdio());
    command.stdout(opts.stdout.to_stdio());
    command.stderr(opts.stderr.to_stdio());

    let mut child = command.spawn().map_err(mlua::Error::external)?;
    let (stdout_handle, stderr_handle) =
        spawn_stream_readers(&mut child, opts.stdout, opts.stderr);

    let child_stdin_opt = child.stdin.take();
    let stdin_handle = if let Some(mut child_stdin) = child_stdin_opt {
        if let Some(stdin_str) = opts.stdin {
            Some(thread::spawn(move || {
                let _ = child_stdin.write_all(stdin_str.as_bytes());
            }))
        } else {
            drop(child_stdin);
            None
        }
    } else {
        None
    };

    let timeout_dur = opts.timeout.map(Duration::from_millis);
    let result = wait_for_exit(child, timeout_dur, stdout_handle, stderr_handle, lua);

    if let Some(h) = stdin_handle {
        let _ = h.join();
    }

    result
}

pub fn lua_system(
    lua: &Lua,
    (cmd_val, opts_val): (Value, Option<Table>),
) -> mlua::Result<Value> {
    let cmd_args = parse_cmd(cmd_val)?;
    let opts = parse_opts(opts_val)?;
    let command = build_command(&cmd_args, &opts);

    if opts.mode == ExecutionMode::Detach {
        spawn_detached(command, &opts, lua)
    } else {
        execute_sync(command, opts, lua)
    }
}

const fn launcher() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn open_target(_: &Lua, target: String) -> mlua::Result<()> {
    if target.trim().is_empty() {
        return Err(mlua::Error::runtime("target must be a non-empty string"));
    }

    let mut command = Command::new(launcher());
    command
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);

    let mut child = command.spawn().map_err(mlua::Error::external)?;
    thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}

pub fn register(lua: &Lua, datamile_tbl: &Table) -> mlua::Result<()> {
    let isatty_fn = lua.create_function(|_, fd: Option<i32>| {
        let fd_num = fd.unwrap_or(0);
        Ok(is_terminal_fd(fd_num))
    })?;

    let sys_table = lua.create_table()?;
    sys_table.set("isatty", isatty_fn)?;
    sys_table.set("open", lua.create_function(open_target)?)?;

    let mt = lua.create_table()?;
    mt.set(
        "__call",
        lua.create_function(|lua, (_, args): (Value, (Value, Option<Table>))| {
            lua_system(lua, args)
        })?,
    )?;
    sys_table.set_metatable(Some(mt))?;

    datamile_tbl.set("system", sys_table)
}
