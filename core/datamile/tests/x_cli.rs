//! `datamile x` asks a live daemon to resolve targets and then runs the action
//! in the CLI process. These tests drive a real daemon in a temporary `$HOME`
//! and check what the CLI prints and exits with.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

/// Integration tests run the built binary, not a library copy of it.
const BIN: &str = env!("CARGO_BIN_EXE_datamile");
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// `Fixture::new` writes exactly this many albums per media type.
const ALBUMS_ON_DISK: usize = 2;

/// A daemon child kills itself when the test ends, even when it panics.
struct Daemon {
    child: tokio::process::Child,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

struct Fixture {
    _root: TempDir,
    config_dir: PathBuf,
    lib: PathBuf,
    home: PathBuf,
    port: u16,
}

impl Fixture {
    /// Two media types share one entity dir each, so the resolver sees a real
    /// cross product: `(Alpha, Beta) x (album, cd)`.
    fn new() -> Self {
        let root = TempDir::new().expect("temp");
        let config_dir = root.path().join("config");
        let lib = root.path().join("albums");
        let home = root.path().join("home");
        let port = free_port();

        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::create_dir_all(&home).expect("home");

        write_album(&lib, "Alpha");
        write_album(&lib, "Beta");
        write(
            &config_dir.join("init.lua"),
            &config(root.path(), &lib, port),
        );

        Self {
            _root: root,
            config_dir,
            lib,
            home,
            port,
        }
    }

    fn album_dir(&self, name: &str) -> String {
        self.lib.join(name).to_string_lossy().into_owned()
    }

    fn missing_dir(&self) -> String {
        self.lib.join("Missing").to_string_lossy().into_owned()
    }

    /// Runs `datamile x <args>` as a child of the daemon's `$HOME`, with stdin
    /// not a terminal so `ctx.isatty` is deterministic.
    async fn x(&self, args: &[&str]) -> Output {
        self.x_in(&self.home.clone(), args).await
    }

    /// The same, with an explicit shell working directory: a scoped action
    /// without `--dir` targets it.
    async fn x_in(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut command = tokio::process::Command::new(BIN);
        command
            .arg("--config")
            .arg(&self.config_dir)
            .arg("x")
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env_remove("DATAMILE_CONFIG_PATH")
            .stdin(Stdio::null());

        command.output().await.expect("run datamile x")
    }

    /// Starts the daemon and waits until both entities of both media types are
    /// indexed, so no test races the first compile pass.
    async fn start_daemon(&self) -> Daemon {
        let child = tokio::process::Command::new(BIN)
            .arg("--config")
            .arg(&self.config_dir)
            .arg("--silent")
            .arg("server")
            .env("HOME", &self.home)
            .env_remove("DATAMILE_CONFIG_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn daemon");

        let daemon = Daemon { child };
        self.wait_until_indexed().await;

        daemon
    }

    async fn wait_until_indexed(&self) {
        let client = local_client();
        let url = format!("http://127.0.0.1:{}/api/media", self.port);
        let deadline = std::time::Instant::now() + READY_TIMEOUT;

        while std::time::Instant::now() < deadline {
            if let Ok(response) = client.get(&url).send().await
                && let Ok(body) = response.json::<Value>().await
                && has_both_media_types(&body)
            {
                return;
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }

        panic!(
            "daemon on port {} never indexed both media types",
            self.port
        );
    }
}

fn has_both_media_types(body: &Value) -> bool {
    let types = &body["types"];

    types["album"] == json!(ALBUMS_ON_DISK) && types["cd"] == json!(ALBUMS_ON_DISK)
}

fn write_album(lib: &Path, name: &str) {
    write(
        &lib.join(name).join("album.toml"),
        &format!("name = \"{name}\"\n"),
    );
}

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mk dir");
    }

    std::fs::write(path, content).expect("write file");
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = listener.local_addr().expect("probe addr").port();
    drop(listener);

    port
}

fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

/// Two media types over the same dirs and one generic action, all writing to
/// the temporary cache and state roots.
fn config(root: &Path, lib: &Path, port: u16) -> String {
    format!(
        r#"
d.config({{
    storage = {{
        cache = "{cache}",
        state = "{state}",
    }},
    server = {{ port = {port} }},
}})

local track = d.media.node("track", {{
    select = function(_ctx) return {{}} end,
}})

local function album_like(name)
    local root = d.media.root(name, {{
        init = "{lib}",
        select = function(dir) return d.fs.roots(dir, "album.toml") end,
        id = function(ctx) return ctx.tags.name end,
        nodes = {{ "track" }},
    }})
    root.tag("name", function(ctx)
        return d.fs.read_toml(ctx.path .. "/album.toml").name
    end)

    return root
end

album_like("album")
album_like("cd")

datamile.action("open_album", {{
    run = function(ctx)
        if ctx.lock == nil then
            print("targetless")
            return
        end

        print(ctx.media .. ":" .. ctx.lock[ctx.media].id .. ":" .. table.concat(ctx.options, ","))
    end,
}})

datamile.action("open_scoped", {{
    media = "album",
    run = function(ctx)
        print("scoped:" .. ctx.media .. ":" .. ctx.lock[ctx.media].id)
    end,
}})

datamile.action("pick", {{
    run = function(ctx)
        local id = ctx.lock[ctx.media].id
        if id == "Alpha" then
            error("boom " .. id)
        end

        print("picked:" .. id)
    end,
}})
"#,
        cache = root.join("cache").display(),
        state = root.join("state").display(),
        lib = lib.display(),
        port = port,
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Without a daemon there is no entity index, so the CLI refuses instead of
/// resolving anything itself.
#[tokio::test]
async fn refuses_to_run_without_a_daemon() {
    let fixture = Fixture::new();

    let output = fixture
        .x(&["open_album", "--dir", &fixture.album_dir("Alpha")])
        .await;

    assert!(!output.status.success(), "expected a non-zero exit");
    assert!(
        stderr(&output).contains("datamile server"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

/// Every `--dir` is paired with every `--media` in argument order, and each run
/// executes in the CLI process.
#[tokio::test]
async fn runs_the_dir_media_cross_product_in_argument_order() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;

    let output = fixture
        .x(&[
            "open_album",
            "--dir",
            &fixture.album_dir("Beta"),
            "--dir",
            &fixture.album_dir("Alpha"),
            "--media",
            "cd",
            "--media",
            "album",
        ])
        .await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    let lines: Vec<&str> = printed.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        lines,
        ["cd:Beta:", "album:Beta:", "cd:Alpha:", "album:Alpha:"]
    );
}

/// A target that fails to resolve reports its error and does not stop the
/// remaining targets.
#[tokio::test]
async fn keeps_running_after_a_failing_target() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;

    let output = fixture
        .x(&[
            "open_album",
            "--media",
            "album",
            "--dir",
            &fixture.album_dir("Alpha"),
            "--dir",
            &fixture.missing_dir(),
        ])
        .await;

    assert!(
        !output.status.success(),
        "expected a non-zero exit; stdout: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("album:Alpha:"),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("does not exist"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}

/// A generic action with no `--dir` gets one targetless run: `ctx.lock` and
/// `ctx.media` are both nil.
#[tokio::test]
async fn runs_a_generic_action_targetless() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;

    let output = fixture.x(&["open_album"]).await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "targetless");
}

/// Everything after `--` reaches `ctx.options` verbatim.
#[tokio::test]
async fn passes_trailing_options_to_the_action() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;

    let output = fixture
        .x(&[
            "open_album",
            "--dir",
            &fixture.album_dir("Alpha"),
            "--media",
            "album",
            "--",
            "-f",
            "x",
        ])
        .await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "album:Alpha:-f,x");
}

/// A scoped action with no `--dir` targets the shell's working directory.
#[tokio::test]
async fn scoped_action_without_a_dir_targets_the_working_directory() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;
    let cwd = fixture.lib.join("Alpha");

    let output = fixture.x_in(&cwd, &["open_scoped"]).await;

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output).trim(), "scoped:album:Alpha");
}

/// A `run(ctx)` that raises fails only its own target, and the CLI exits `1`
/// after the remaining targets ran.
#[tokio::test]
async fn keeps_running_after_a_failing_run() {
    let fixture = Fixture::new();
    let _daemon = fixture.start_daemon().await;

    let output = fixture
        .x(&[
            "pick",
            "--media",
            "album",
            "--dir",
            &fixture.album_dir("Alpha"),
            "--dir",
            &fixture.album_dir("Beta"),
        ])
        .await;

    assert!(!output.status.success(), "expected a non-zero exit");
    assert!(
        stdout(&output).contains("picked:Beta"),
        "stdout: {} stderr: {}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("boom Alpha"),
        "unexpected stderr: {}",
        stderr(&output)
    );
}
