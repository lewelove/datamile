# Actions: media-scoped action contract

Status: specification.

`specs/multi-locks/spec.md` defines `EntityKey` and target resolution; this
document defines `d.action`, the derived action map, `ctx`, `POST /api/x`, LAN
origin/host enforcement, and `datamile x`.

## Intent

Today the action registry is a flat `HashMap<String, ActionDef>` where later
declarations merge into earlier ones, `ctx` exposes `{ items, options, isatty }`
with string-matched options, and `interfaces/web-app` hardcodes `type: "album"`.
Actions should declare which media types they belong to, support per-media
presentation (`label` and `description`), receive the literal parsed lock JSON
and resolved media type, and execute through one unified `/api/x` contract.

## Goals

- Declare each action once per canonical name (`-` -> `_`), with optional
  `media` scope (`nil`, string, or string list) and optional per-media
  `label`/`description`.
- Pass `ctx = { lock, media, options, isatty }` to `run(ctx)`.
- Serve both HTTP clients and `datamile x` through one `POST /api/x` endpoint
  parameterized by `execute: bool`, with `datamile x` requiring a running daemon.
- Enforce LAN-capable `server.host` binding alongside `Host` and `Origin`
  protection across all mutating `/api/*` routes.

## Non-goals

- Supporting `datamile x` without a running `datamile server` daemon.
- Changing the logic manifest (`filters`, `groupers`, `orders`, `libraries`,
  `shelves`, `cabinets`) to the action map shape.
- Making `interfaces/web-app` render entities of arbitrary non-album media types
  beyond passing the rendered entity's media type and `path`.

## Core model

- `media = nil` -> generic action, not bound to a specific media root.
- `media = "album"` -> scoped to one media type.
- `media = {"album", "movie"}` -> multi-purpose action scoped to several media
  types.
- `label` / `description`: `nil`, a non-empty string, or `function(media)`
  returning a non-empty string.
- `run`: `function(ctx)` taking one argument.
- `ctx`: `{ lock, media, options, isatty }`. `ctx.lock` is the parsed
  `<dir>/<media>.lock.json` JSON (`nil` when targetless), and `ctx.media` is the
  resolved media-type string (`nil` only when targetless).
- At execution time, `media` governs UI visibility and default media selection
  when `--media` is omitted; an explicit `--media` selects `<dir>/<media>.lock.json`
  literally for any registered media type.

```lua
d.action("open_terminal", {
  media = { "album", "movie" },
  label = function(media)
    return media == "album" and "Open Album Terminal" or "Open Movie Terminal"
  end,
  description = function(media)
    return "Open a terminal in the " .. media .. " directory"
  end,
  run = function(ctx)
    d.system({ "alacritty" }, { cwd = ctx.lock[ctx.media].location.path, detach = true })
  end,
})

d.action("open_config_in_terminal", {
  label = "Open Config in Terminal",
  run = function(ctx)
    d.system({ "alacritty" }, { cwd = "~/.config/datamile", detach = true })
  end,
})
```

## Changes

### 1. Canonical names

`d.action(name, def)` normalizes `-` -> `_` on `name` (the canonical key used
by `REGISTRY.actions`, `datamile x`, and `POST /api/x`) and rejects duplicate
canonical names at declaration time:

```lua
d.action("open-terminal", { media = "album", run = album_terminal })
d.action("open_terminal", { media = "movie", run = movie_terminal })
-- error: action 'open_terminal' is declared twice (canonicalized from 'open-terminal')
```

`run` is required on every declaration. The existing `run = t.run or existing.run`
merge is removed; `label` defaults to `name`, `description` defaults to `nil`,
and the unread `config` field is dropped.

### 2. Declaration validation and presentation resolution

Shape validation runs in `__DATAMILE_GET_ACTIONS`, returning
`ActionDeclarations` on `EvaluatedDomainData.actions`:

- `name` must be a non-empty string.
- `media` must be `nil`, a non-empty string, or a non-empty list of non-empty
  strings (repeated entries are deduplicated). Empty strings or empty tables
  fail config load (`action '<name>': 'media' must be a non-empty string or list of non-empty strings`).
- `run` must be a function.
- `label` and `description` must each be `nil`, a non-empty string, or a
  function.

Media validation runs in Rust inside `validate_media_schema` (`&MediaManifest,
&LogicManifest, &ActionDeclarations`), calling `require_media` for every entry
in `media`:

```console
d.action("open_book", { media = "book", run = print })
error: action 'open_book' declares unknown media type 'book'
```

After `validate_media_schema` succeeds, Rust calls
`DomainLuaEngine::resolve_actions(&self, decls: &ActionDeclarations) -> Result<ActionMap>`
(invoking `__DATAMILE_RESOLVE_ACTIONS`) to evaluate `label(m)` and
`description(m)` callbacks for each verified media type:

- A callback returning `nil`, `""`, or a non-string, or raising a Lua error,
  fails config load with the action name and media type
  (`action '<name>': '<field>' callback failed for media '<m>'`).
- When `label` is `nil`, it falls back to the canonical action name; when
  `description` is `nil`, it is omitted (`None`).

### 3. `ctx` contract

`ActionCtx` in `core/datamile-lua-domain/src/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCtx {
    pub lock: Option<serde_json::Value>,
    pub media: Option<String>,
    pub options: Vec<String>,
    pub isatty: bool,
}
```

`DomainLuaEngine::execute_action(&self, name: &str, ctx: &ActionCtx)` converts
`ctx` to Lua and invokes `run(ctx)`:

- `lock`: parsed `<dir>/<media>.lock.json` JSON (`None` -> `nil` when targetless).
- `media`: resolved media-type string (`None` -> `nil` when targetless).
- `options`: string array built from trailing `-- <options>...` arguments (or
  `request.options` on HTTP).
- `isatty`: `std::io::stdin().is_terminal()` in the CLI process; `false` when
  executed by the daemon (`execute: true`).

### 4. Derived action map

A single `ActionMap = BTreeMap<String, ActionEntry>` replaces
`HashMap<String, ActionDef>` across `ResolvedDomainConfig`, `ServerConfig`,
`INIT`, and `LOGIC_UPDATE`. `EvaluatedDomainData.actions` holds
`ActionDeclarations`, which callers resolve into `ActionMap` via
`DomainLuaEngine::resolve_actions` after `validate_media_schema`. The unused
`MediaState.actions` field is removed.

```rust
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionPresentation {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionEntry {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<BTreeMap<String, ActionPresentation>>,
}

pub type ActionMap = BTreeMap<String, ActionEntry>;
```

Serialized wire shape on `INIT` and `LOGIC_UPDATE`:

```json
"actions": {
  "open_config_in_terminal": {
    "label": "Open Config in Terminal",
    "description": "Open terminal inside Datamile configuration directory"
  },
  "open_terminal": {
    "label": "open_terminal",
    "media": {
      "album": {
        "label": "Open Album Terminal",
        "description": "Open a terminal in the album directory"
      },
      "movie": {
        "label": "Open Movie Terminal",
        "description": "Open a terminal in the movie directory"
      }
    }
  }
}
```

Client lookup (`interfaces/web-app`):

- `hasAction(name, mediaType)`: returns `true` if `entry.media` is absent
  (generic) or if `mediaType` is non-null and `mediaType in entry.media`.
- `actionPresentation(name, mediaType)`: returns `entry.media[mediaType]` when
  scoped, or `{ label: entry.label, description: entry.description }` when
  generic.
- `core/datamile-media/src/ingest.rs` adds `"path": root` to each entity's
  `dict` (matching `GET /api/media/{type}`), and `AlbumDictEntry.path` carries
  it synchronously to the web app.

### 5. Unified `POST /api/x` endpoint

`POST /api/x` replaces `POST /api/actions/{name}` and `POST /api/select`. Both
`interfaces/web-app` and `datamile x` send the same JSON body:

```rust
#[derive(Debug, Deserialize)]
pub struct XRequest {
    pub name: String,
    #[serde(default)]
    pub dirs: Vec<String>,
    #[serde(default)]
    pub media: Vec<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default = "default_true")]
    pub execute: bool,
}

#[derive(Debug, Serialize)]
pub struct XRun {
    pub media: Option<String>,
    pub dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct XResponse {
    pub runs: Vec<XRun>,
}
```

Request rules:

- `name` is canonicalized (`-` -> `_`). Unknown actions return `404 Not Found`
  (`{ "error": String }`).
- Every entry in `media` must be registered in `MediaManifest`; unregistered
  names fail before directory resolution with `400 Bad Request`.
- Every entry in `dirs` must be an already-expanded path sent by the caller (the
  CLI expands `~` and `.` locally before sending); relative or unexpanded paths
  are rejected with `400 Bad Request`.
- Empty `dirs` with non-empty `media`, or empty `dirs` on a scoped action,
  returns `400 Bad Request`.
- Empty `dirs` and empty `media` on a generic action resolves one targetless run
  (`media: None, dir: None`).
- Browser detection: if a request carries `Origin` or `Sec-Fetch-Site` headers
  with `execute: false`, return `400 Bad Request`
  (`"execute=false is restricted to CLI callers"`).
- Non-JSON `Content-Type` returns `415 Unsupported Media Type`; malformed JSON
  returns `422 Unprocessable Entity` (Axum `Json<XRequest>`).
- When `execute: true` (web/HTTP clients): the daemon reads each resolved
  entity's `<dir>/<media>.lock.json` from disk, invokes `run(ctx)` with
  `isatty: false`, omits `lock` in `XRun`, and records any target or Lua runtime
  error in `XRun.error`.
- When `execute: false` (`datamile x`): the daemon resolves targets, reads each
  target's `<dir>/<media>.lock.json` into `XRun.lock`, and skips `run(ctx)`.

### 6. CLI `datamile x`

Usage: `datamile x <name> [--dir <entity-dir>...] [--media <media_type>...] [-- <options>...]`

- `--type`, `--directory`, `--query`, `--playing`, `--id`, `--recursive`, and
  `--library` are removed. Trailing arguments after `--` populate `ctx.options`
  verbatim.
- `datamile x` requires a live daemon: it probes `InstanceLock::holder()` and
  checks `GET /api/identity` (matching canonical `config_path`, `version`, and
  `reload_error == None`). If no daemon holds `InstanceLock`, it exits non-zero
  with `error: datamile daemon is not running; start it with 'datamile server'`.
- `datamile x` evaluates `init.lua` directly via `DomainLuaEngine` (the same
  lightweight loader as `core/datamile/src/interface.rs`, without calling
  `reload_config_and_logic`, `discover_roots`, or `scan_library`).
- Before calling `POST /api/x`, the CLI canonicalizes every `--dir` locally
  (expanding `~` and resolving relative paths against the shell's working
  directory). When `--dir` is omitted, the CLI checks the action's declaration:
  generic actions send `dirs: []`; scoped actions send `[canonical_cwd]`.
- `datamile x` posts `{ name, dirs, media, options, execute: false }` to
  `/api/x`.
- For `datamile x intermediary`: prints `runs` mapped to
  `[{ lock, media, options, isatty, error? }]` as JSON to stdout and exits `1`
  if any entry has `error`.
- For all other actions: executes `run(ctx)` locally in the CLI process via
  `DomainLuaEngine::execute_action` for every `XRun` where `error` is `None`,
  setting `ctx.isatty = std::io::stdin().is_terminal()`. Errors from `XRun.error`
  or local `run(ctx)` are printed to stderr; remaining runs proceed, and the CLI
  exits `1` if any run failed.

### 7. LAN binding, host check, and origin policy

`datamile-server` binds `config.host` (`IpAddr`) and `config.port` from
`ServerConfig` (instead of hardcoding `127.0.0.1`), and reads
`server.allowed_origins: Vec<String>` from `d.config({ server = { ... } })`:

- Built-in allowlist: loopback (`localhost`, `127.0.0.1`, `[::1]`) and
  `192.168.0.0/16`, ignoring ports. User entries in `server.allowed_origins`
  (hostnames or `ip/prefix` CIDR strings, validated at config load via
  `std::net::IpAddr`) extend this allowlist. Malformed entries fail config load
  with the offending string. Host/origin port stripping must preserve IPv6
  brackets (`[::1]:8000` -> `[::1]`).
- One middleware runs across all `/api/*` mutating routes (`POST`/`PUT`/`DELETE`),
  including `POST /api/x`, `/api/reset`, `/api/update`, `/api/state`,
  `/api/media/*/update/*`, and `/api/playback/*`:
  - **Host header check**: the request `Host` (port stripped) must match
    loopback, the server's bound IP/hostname, or an allowed hostname/CIDR entry.
    Mismatched or missing `Host` returns `403 Forbidden` (blocks DNS rebinding).
  - **Origin header check**: if `Origin` is present, `Origin: null` is rejected
    with `403 Forbidden`, and any origin host/IP outside the allowlist is
    rejected with `403 Forbidden` (blocks cross-origin simple requests on
    body-less POST endpoints).
- `CorsLayer` enforces the same allowlist predicate (`allow_methods`,
  `allow_headers` for `Content-Type`).

## Edge cases

Target resolution rules and the full `--dir` / `--media` matrix are defined in
`specs/multi-locks/spec.md` (*Target resolution* and *Edge cases*). Additional
action-layer cases:

| Case | Behaviour |
| --- | --- |
| `POST /api/x` with `execute: false` and `Origin` or `Sec-Fetch-Site` header | `400 Bad Request` (`execute=false is restricted to CLI callers`) |
| `POST /api/x` with non-JSON `Content-Type` | `415 Unsupported Media Type` |
| `POST /api/x` with malformed JSON body | `422 Unprocessable Entity` |
| `POST /api/x` with relative or `~` path in `dirs` | `400 Bad Request` |
| `POST /api/x` with `execute: true` and `run(ctx)` raises Lua error | `200 OK` with `runs[i].error` populated |
| `POST /api/x` (or any mutating `/api/*` route) with unknown `Host` | `403 Forbidden` |
| `POST /api/x` (or any mutating `/api/*` route) with `Origin: null` or public `Origin` | `403 Forbidden` |
| `datamile x` invoked while `datamile server` is not running | exits `1` naming `datamile server` |
| `d.action` `label` or `description` callback returns `nil` or `""` | config load fails naming action and media type |

## Alternatives Considered

- **Guard execution on `action.media`**: Refusing `--dir X --media Y` when an
  action did not declare `Y` broke generic actions on typed entities and blocked
  running tools across newly added lock types. Keeping `action.media` for
  validation, UI presentation, and default selection when `--media` is omitted
  while treating explicit `--media` as a literal lock selector preserves safety
  without blocking explicit commands.
- **Keep `datamile x` working offline via `state.json`**: Maintaining a parallel
  offline resolver required re-evaluating `init.lua`, parsing full `state.json`
  fingerprints on every CLI call, and risked online/offline drift. Making
  `datamile server` load-bearing for `datamile x` leaves one resolver and one
  source of truth.
- **Daemon PTY forwarding instead of `execute: bool`**: Interactive actions
  (`embed`, `rename`, `lyrics`) open `/dev/tty` and rely on `stdio = "inherit"`
  and caller `cwd`. Returning resolved runs for local CLI execution when
  `execute: false` preserves native terminal behaviour without a PTY proxy.
- **Drop `server.host` binding / LAN support**: Keeping `127.0.0.1`-only binding
  prevents direct access from LAN devices. Binding `config.host` and enforcing
  `Host` + `Origin` checks across all mutating endpoints supports LAN safely.

## Risks & Limits

- **LAN trust assumption**: Any device or webpage served inside
  `192.168.0.0/16` or `server.allowed_origins` can invoke configured actions
  (`d.system`). Users binding `0.0.0.0` on untrusted networks should narrow
  `allowed_origins` or bind `127.0.0.1`.
- **Daemon-spawned actions (`execute: true`)**: Targetless or UI-triggered
  actions run in the daemon's environment and cwd; actions launching terminals
  or GUI apps require display environment variables on the daemon service unit.
- **Breaking config and CLI migration**: Existing `actions.lua` scripts must
  update `ctx.items` -> `ctx.lock[ctx.media]` and string `ctx.options` ->
  array `ctx.options`:

  ```lua
  -- before:
  local is_force = ctx.options:find("%-f") or ctx.options:find("%-%-force")
  for _, entry in ipairs(ctx.items) do ... entry.path ... end

  -- after:
  local is_force = d.list.contains(ctx.options, "-f") or d.list.contains(ctx.options, "--force")
  local path = ctx.lock[ctx.media].location.path
  ```

## Implementation

Cross-spec landing order: `specs/multi-locks/spec.md` phases 1–3 land before
(or together with) phases 2–3 here so `EntityKey` and multi-owner `LogicEngine`
lookups exist when the shared `POST /api/x` resolver lands.

### Files

| Action | Path | Change |
| --- | --- | --- |
| Modify | `core/datamile-lua-domain/src/config.lua` | Canonicalize `-` -> `_`, reject duplicates, drop `config` field, store raw declarations |
| Modify | `core/datamile-lua-domain/src/core.lua` | Validate declaration shapes in `__DATAMILE_GET_ACTIONS`; evaluate `label`/`description` callbacks in `__DATAMILE_RESOLVE_ACTIONS` |
| Modify | `core/datamile-lua-domain/src/config.rs` | Add `ActionDeclarations`, `ActionPresentation`, `ActionEntry`, `ActionMap`, `ActionCtx`; add `allowed_origins: Vec<String>` to `ServerConfig` |
| Modify | `core/datamile-lua-domain/src/engine.rs` | `EvaluatedDomainData.actions` holds `ActionDeclarations`; `ResolvedDomainConfig.actions` becomes `ActionMap`; add `resolve_actions`; update `execute_action(&self, name: &str, ctx: &ActionCtx)` |
| Modify | `core/datamile-lua-domain/src/lib.rs` | Re-export `ActionDeclarations`, `ActionEntry`, `ActionMap`, `ActionPresentation`, `ActionCtx` |
| Modify | `core/datamile-lua-domain/src/tests.rs` | Update action tests for `ActionMap`, `ActionCtx`, duplicate canonical names, and callback validation |
| Modify | `core/datamile-media/src/schema.rs` | Add action media validation to `validate_media_schema` using `require_media` |
| Modify | `core/datamile-media/src/ingest.rs` | Include `"path": root` in entity `dict` |
| Modify | `core/datamile-media/src/state.rs` | Remove `MediaState.actions` |
| Modify | `core/datamile-media/src/backend.rs` | Call `resolve_actions` after `validate_media_schema`; pass `server.host` and `server.allowed_origins` in `ServerConfig`; remove `ActionTargetQuery` impl (`resolve_action_targets`, `param_targets`, `playing_target`, `item_for`); update `execute_action` signature |
| Modify | `core/datamile-media/src/watcher.rs` | Broadcast `ActionMap` in `LOGIC_UPDATE` |
| Modify | `core/datamile-media/src/tests.rs` | Update `validate_media_schema` calls (7 sites) and `MediaState` literals (3 sites); add action schema & `dict.path` tests |
| Modify | `core/datamile-server/src/state.rs` | Add `host: String` and `allowed_origins: Vec<String>` to `ServerConfig`; type `ServerConfig.actions` as `ActionMap` |
| Modify | `core/datamile-server/src/backend.rs` | Remove `ActionTargetQuery` and `resolve_action_targets`; update `execute_action(&self, name: &str, ctx: &ActionCtx)` |
| Modify | `core/datamile-server/src/lib.rs` | Bind parsed `config.host`; install `Host`/`Origin` middleware and `CorsLayer`; re-export `ActionEntry`, `ActionMap`, `ActionCtx` |
| Create | `core/datamile-server/src/api/x.rs` | Implement `POST /api/x` (`XRequest`, `XResponse`, `XRun`) delegating to shared target resolver |
| Delete | `core/datamile-server/src/api/actions.rs` | Replaced by `api/x.rs` |
| Delete | `core/datamile-server/src/api/select.rs` | Replaced by `api/x.rs` |
| Modify | `core/datamile-server/src/api/mod.rs` | Wire `/api/x`, remove `/api/actions/{name}` and `/api/select` |
| Modify | `core/datamile-server/src/api/websocket.rs`, `system.rs` | Serialize `ActionMap` directly |
| Modify | `core/datamile-server/src/tests.rs` | Add `Host: 127.0.0.1:8000` to existing POST test requests; update `MockBackend` and `ServerConfig` fixtures; add `/api/x` and `Host`/`Origin`/CORS tests |
| Modify | `core/datamile/src/main.rs` | Narrow `XArgs` (`--dir`, `--media`, trailing `--` options); require live daemon; load config via `DomainLuaEngine`; resolve `.`/`~` locally; call `POST /api/x` with `execute: false`; run `execute_action` locally or print `intermediary` |
| Modify | `core/datamile/Cargo.toml` | Add `assert_cmd` / `tempfile` dev-dependencies if needed for CLI tests |
| Create | `core/datamile/tests/x_cli.rs` | CLI integration tests against a live test server instance |
| Create | `interfaces/web-app/src/library/actions.ts` | Pure TypeScript `hasAction(actions, name, mediaType)` and `actionPresentation(actions, name, mediaType)` helpers |
| Create | `interfaces/web-app/src/library/actions.test.ts` | `bun test` unit tests for `hasAction` and `actionPresentation` |
| Modify | `interfaces/web-app/src/types.ts`, `api.ts`, `library/collection.svelte.ts` | `ActionEntry`/`ActionMap` types, `AlbumDictEntry.path`, `executeAction(name, media, dir, options?)` targeting `POST /api/x` |
| Modify | `interfaces/web-app/src/modules/HomeView/ModalDrawer/ModalDrawer.svelte`, `modules/NavigationBar.svelte`, `App.svelte` | Update all 8 `hasAction` and 5 label/execute call sites to pass `media` and `path` |
| Modify | `interfaces/web-app/package.json`, `devenv.nix` | Add `"test": "bun test"` script and run it in `devenv shell -- tests` |
| Modify | `GLOSSARY.md`, `docs/lua-api.md`, `README.md` | Update full API docs (§2, §9, §10, §11) and `README.md` action examples after implementation |

### Phases

Each phase must be a separate commit that compiles cleanly and passes
`devenv shell -- lint` and `devenv shell -- tests`:

1. **Lua & schema**: `d.action` canonicalization and collision check,
   `__DATAMILE_GET_ACTIONS` -> `ActionDeclarations`, `__DATAMILE_RESOLVE_ACTIONS`
   via `resolve_actions`, `ActionPresentation`/`ActionEntry`/`ActionMap`/`ActionCtx`
   in `datamile-lua-domain`, `validate_media_schema` action check,
   `ServerConfig.allowed_origins` parsing/CIDR validation, `ingest.rs` `dict.path`,
   and `backend.rs` projecting `ActionMap` into existing `ActionDef` so
   downstream crates keep compiling for this commit.
2. **Routes, `ActionMap` cutover, & LAN security**: Switch `ServerConfig.actions`
   and `execute_action` to `ActionMap`/`ActionCtx` across `datamile-media`,
   `datamile-server`, and `main.rs`, delete `MediaState.actions`, `actions.rs`,
   `select.rs`, and `ActionTargetQuery`, add `api/x.rs` (`POST /api/x`), bind
   `config.host`, install `Host`/`Origin` middleware + `CorsLayer`, and update
   `datamile-server/src/tests.rs`.
3. **CLI `datamile x` & integration tests**: Daemon-required check
   (`InstanceLock` + `/api/identity`), lightweight `DomainLuaEngine` config
   evaluation (matching `interface.rs`), local `--dir` canonicalization and cwd
   fallback, `POST /api/x { execute: false }`, local `run(ctx)` execution,
   `intermediary` JSON array output, `core/datamile/tests/x_cli.rs`.
4. **Web app & docs**: `ActionMap` store, pure `actions.ts` lookup module +
   `bun test` coverage, updated `ModalDrawer.svelte`/`NavigationBar.svelte`/`App.svelte`
   call sites, `docs/` + `README.md` updates.

### Tests

- `datamile-lua-domain` (`core/datamile-lua-domain/src/tests.rs`):
  - Duplicate canonical name (`foo-bar` vs `foo_bar`) errors at declaration time.
  - `media = ""` and `media = {}` error; repeated entries in `media` deduplicate.
  - `label` / `description` callback raising or returning `nil`/`""` errors with action name and media type.
  - Resolved `ActionMap` contains generic entry when `media = nil` and per-media entries under `media` when scoped; unused `config = {}` is ignored.
  - `execute_action` passes `{ lock, media, options, isatty }` to `run(ctx)`.
  - Malformed `server.allowed_origins` CIDR/host fails config evaluation naming the bad entry.
- `datamile-media` (`core/datamile-media/src/tests.rs`):
  - Unknown media in `d.action` fails in `validate_media_schema` with `"declares unknown media type"`.
  - `ingest.rs` entity `dict` includes `"path"`.
- `datamile-server` (`core/datamile-server/src/tests.rs`):
  - `POST /api/x` returns `200 { runs }` for targetless generic (`execute: true` and `false`), scoped multi-dir/multi-media cross-product, and per-run Lua errors (`runs[i].error`).
  - `POST /api/x` returns `400` on relative/unexpanded `dirs`, missing `dirs` for scoped action, `execute: false` with `Origin` or `Sec-Fetch-Site` header, or unregistered `media`; `404` on unknown action; `415` on non-JSON; `422` on malformed JSON.
  - `Host` / `Origin` middleware + `CorsLayer` allows `localhost`, `127.0.0.1`, `[::1]`, `[::1]:8000`, `192.168.0.0/16` (any port), and configured `allowed_origins`; returns `403` for `Host: evil.com:8000`, `Origin: null`, and `Origin: https://example.com` across `/api/x`, `/api/reset`, and `/api/playback/clear`.
  - `INIT` and `LOGIC_UPDATE` broadcast `ActionMap`.
- `datamile` (`core/datamile/tests/x_cli.rs`):
  - Fails non-zero naming `datamile server` when daemon is not running.
  - Runs `(dir, media)` cross-product in argument order; non-zero exit when any run fails while remaining runs still execute.
  - Targetless generic run receives `ctx.lock = nil, ctx.media = nil`; `intermediary` prints resolved `ctx` array as JSON.
- `interfaces/web-app` (`interfaces/web-app/src/library/actions.test.ts`):
  - `hasAction` and `actionPresentation` return scoped entries for matching `mediaType`, return generic entries for any `mediaType`, and reject actions scoped only to other media types.
