# Actions: media-scoped action contract

Status: proposed. Not approved. Records the feedback from the 2026-11-24
session and removes the execution guard so `--media` acts purely as a lock
selector.

## Intent

Datamile must stay media-agnostic. Today the action registry is one flat
namespace and the web app hardcodes `type: "album"`. An action should declare
which media roots it belongs to (for validation, UI presentation, and default
selection when `--media` is omitted), present a different label and description
per media type, and receive both the literal lock JSON and resolved media type.

## Terms

Every word here is defined in `docs/vocabulary.md`: media type, media root,
`init`, media entity, entity dir, id, entity manifest, media manifest, logic
manifest, lock, entity index, `REGISTRY`, getter, `location.path`, and `ctx`.
`EntityKey` and `datamile x` target resolution are defined in
`specs/multi-locks/spec.md`.

## The model

- `media = nil` -> generic. Not tied to a specific media root. `nil` means no
  `media` key was declared; it never falls back to `"album"`.
- `media = "album"` -> one media type.
- `media = {"album", "movie"}` -> several media types.
The key of `actions.media`, every value of `--media`, `ctx.media`, and the
single top-level key of a lock file are the same media-type string.

- `label` / `description`: `nil`, a non-empty string, or
  `function(media)` returning a non-empty string.
- `run`: `function(ctx)` taking one argument.
- `ctx`: `{ lock, media, options, isatty }`. `ctx.lock` is the literal parsed
  `<dir>/<media>.lock.json` JSON (`WYSIWYG`), and `ctx.media` is the resolved
  media-type string (`nil` only when no entity was targeted).

```lua
d.action("open_terminal", {
  media = {"album", "movie"},
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

## Decisions

### 1. One declaration per canonical name; collisions fail config load

`d.action(name, ...)` normalizes `-` -> `_` on `name` (the canonical key used
by the registry, `datamile x`, `/api/x`, and `/api/x/resolve`), stores without
merging, and rejects any duplicate canonical name at call time:

```lua
-- actions/album.lua
d.action("open-terminal", { media = "album", run = album_terminal })
-- actions/movie.lua
d.action("open_terminal", { media = "movie", run = movie_terminal })
-- error: action 'open_terminal' is declared twice (canonicalized from 'open-terminal')
```

Consequence: `run` is required on every declaration. The current
`run = t.run or existing.run` merge goes away, and `label` / `description`
default to `nil` instead of inheriting.

### 2. Resolution happens after `init.lua`, not in `d.action`

`d.action` only stores the declaration. `__DATAMILE_GET_ACTIONS` (called after
`init.lua` returns, once both actions and `REGISTRY.media_roots` are final)
produces `ActionSet` from the declarations:

- `media = nil` -> one entry in `generic`.
- `media = "album"` -> one entry under `media.album`.
- `media = {"album", "movie"}` -> one entry under each declared media.

```lua
-- album.lua   d.media.root("album", ...)
-- movie.lua   d.media.root("movie", ...)
-- actions.lua d.action("open_config_in_terminal", { run = ... })  -- media = nil
```

### 3. Validation (media checked before callbacks run)

Shape, in `__DATAMILE_GET_ACTIONS`:

- `name` must be a non-empty string.
- `media` must be `nil`, a non-empty string, or a table of non-empty strings.
  An empty string or an empty table is an error.
- Repeated entries in one `media` list are deduplicated.
- `run` is required and must be a function.
- `label` / `description` must be `nil`, a non-empty string, or a function
  returning a non-empty string. A function returning `nil` or `""` is an error
  (there is no "hide this action for this media").
- `label` `nil` falls back to the canonical action name; `description` `nil` is
  omitted.
- Before calling `label(m)` or `description(m)`, `__DATAMILE_GET_ACTIONS`
  verifies `REGISTRY.media_roots[m] ~= nil` (skipping callbacks for unknown
  media types so Rust's `validate_media_schema` reports the clean schema error
  instead of a Lua traceback inside the user's callback).

Media names are checked in Rust inside `validate_media_schema`
(`&MediaManifest, &LogicManifest, &ActionSet`), reusing `require_media`:

```console
d.action("open_book", { media = "book", run = print })   -- no book root
error: action 'open_book' declares unknown media type 'book'
```

### 4. `ctx = { lock, media, options, isatty }`

`ctx` holds:

- `lock`: the literal parsed `<dir>/<media>.lock.json` object (1-to-1 with
  the file on disk and `GET /api/media/{type}/{id}`), or `nil` when no entity
  was targeted.
- `media`: the resolved/selected media-type string (even for generic actions
  when targeting an entity, e.g. `"vinyl"`), or `nil` when targetless.
- `options`: an array of strings built from everything after `--` (e.g.
  `ctx.options = { "--how", "--fast" }`). `--query` is removed.
- `isatty`: boolean (`true` when CLI `stdin` is a terminal; `false` on HTTP).

`run(ctx)` always takes one argument. The Lua registry keeps one `run`
function per canonical action name; the per-media projection materializes only
`{ label, description }`.

### 5. Client shape: `generic` bucket plus nested `media`

```json
"actions": {
  "generic": {
    "open_config_in_terminal": { "label": "...", "description": "..." }
  },
  "media": {
    "album": {
      "open_terminal": { "label": "Open Album Terminal", "description": "..." }
    },
    "movie": {
      "open_terminal": { "label": "Open Movie Terminal", "description": "..." }
    }
  }
}
```

Client lookup (`mediaType` is the media type of the entity being rendered):

```text
action(name, mediaType):
  if mediaType is not null and actions.media[mediaType] has name: return it
  return actions.generic[name] or nothing
```

A name in `media[mediaType]` never exists in `generic` because canonical action
names are unique across declarations.

### 6. Rust structs, routes, and CORS policy

```rust
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionDef {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ActionSet {
    pub generic: BTreeMap<String, ActionDef>,
    pub media: BTreeMap<String, BTreeMap<String, ActionDef>>,
}
```

`ActionDef` and `ActionSet` live in `core/datamile-lua-domain/src/config.rs`
and are re-exported by `datamile-server`. `EvaluatedDomainData.actions`,
`ResolvedDomainConfig.actions`, and `ServerConfig.actions` all hold `ActionSet`;
the unused `MediaState.actions` field is deleted, and `websocket.rs` /
`system.rs` / `watcher.rs` serialize `ActionSet` directly without conversion.

Two routes replace `POST /api/actions/{name}` and `POST /api/select`. Both
canonicalize `name` (`-` -> `_`), canonicalize `dir`, and delegate resolution to
the same shared Rust resolver defined by `specs/multi-locks/spec.md`
(*Target resolution*):

1. **`POST /api/x`** (HTTP / web-app clients — daemon executes the action):
   - Request (`application/json`):
     `{ "name": String, "dir": Option<String>, "media": Option<String>, "options": Vec<String> }`
     (`options` defaults to `[]`).
   - Target rules:
     - `dir: null, media: null` + generic action -> runs once targetless
       (`ctx.lock = nil, ctx.media = nil`).
     - `dir: null, media: null` + scoped action -> `400 Bad Request` (`dir` is
       required for scoped actions).
     - `dir: null, media: Some(_)` -> `400 Bad Request` (`media` without `dir`).
     - `dir: Some(d), media: Some(m)` -> resolves `(canon(d), m)` literally (no
       guard).
     - `dir: Some(d), media: None` -> Option B resolution on `canon(d)`.
   - Responses:
     - `200 OK`: `{ "status": "ok", "media": Option<String>, "dir": Option<String> }`
     - `400 Bad Request`: `{ "error": String }` (missing/partial target, invalid `media`)
     - `404 Not Found`: `{ "error": String }` (unknown action, or `(dir, media)` not in index)
     - `500 Internal Server Error`: `{ "error": String }` (Lua runtime error)

2. **`POST /api/x/resolve`** (CLI target resolution — CLI executes locally):
   - Request (`application/json`):
     `{ "name": String, "dirs": Vec<String>, "media": Vec<String> }`
     (empty `dirs` + empty `media` on a generic action = targetless; empty `dirs`
     with scoped action or non-empty `media` defaults `dirs` to `["."]`).
   - Response (`200 OK`):
     `{ "runs": [{ "media": Option<String>, "dir": Option<String>, "lock": Value /* null when targetless */, "error": Option<String> }] }`
     (or `400` / `404` `{ "error": String }` for unknown action or unregistered
     `media` before dir resolution).
   - `datamile x` requests `/api/x/resolve` when the daemon holds `InstanceLock`
     (or resolves locally via `init.lua` + `state.json` when offline, without
     `discover_roots` or `scan_library`), then runs `execute_action` **in the
     CLI process** so `isatty`, `/dev/tty` prompts, and `stdio = "inherit"` work
     identically online and offline.
   - `datamile x intermediary` prints the resolved `ctx` array as JSON.

**CORS & origin policy (`core/datamile-server/src/lib.rs`, `datamile-lua-domain/src/config.rs`)**:
- Add `allowed_origins: Vec<String>` to `ServerConfig` (`d.config({ server = { allowed_origins = { ... } } })`), accepting hostnames and `ip/bit_mask` CIDR notation.
- Built-in defaults always allow loopback (`localhost`, `127.0.0.1`, `[::1]`)
  and `192.168.0.0/16`, **ignoring ports** (so Vite-proxied LAN devices and
  same-LAN clients continue to work). User entries in `server.allowed_origins`
  **extend** these defaults (e.g. `"10.0.0.0/8"`, `"box.local"`).
- `CorsLayer` rejects origins outside the allowed set, and `POST /api/x`
  requires `application/json`, preventing arbitrary public websites from making
  drive-by requests to `d.system`.

### 7. No `config` field

`d.action` previously stored `config = t.config or existing.config or {}` and
nothing read it. `ActionDef` carries `label` and `description` only, and the
unused `config` field is dropped. Passing `config` in a declaration is ignored.

## Invocation

`datamile x <name> [--dir <entity-dir>...] [--media <media_type>...] [-- <options>...]`

- `--type` becomes `--media`, `--directory` becomes `--dir`, and `--query`,
  `--playing`, `--id`, `--recursive`, and `--library` are dropped. Options come
  exclusively from after `--`.
- Target resolution, Option B inference when `--media` is omitted,
  canonicalization, offline `state.json` parity, cross-product execution, and
  exit code are defined in `specs/multi-locks/spec.md` (*Target resolution*).
- `--media` is purely a selector of `<dir>/<media>.lock.json`—never an execution
  guard. `ctx.media` receives the resolved media type literally (`nil` only
  when targetless).

## Alternatives Considered

- **Guard execution on `action.media`**: Refusing `--dir X --media Y` when an
  action didn't declare `Y` broke generic actions on typed drawers and prevented
  running existing tools across newly added lock shapes. Using `action.media`
  for validation, UI menus, and Option B default filtering—while treating
  explicit `--media` as a literal selector—keeps defaults safe without blocking
  explicit commands.
- **`ctx = { entity, media }` + `run(ctx)` vs `ctx = { lock }` + `run(ctx, media)`**:
  Dropping the second `run` argument removes four layers of trait churn, while
  keeping `ctx.lock` (literal lock JSON) preserves 1-to-1 WYSIWYG parity with
  lock files on disk and `/api/media/{type}/{id}`. Adding `ctx.media` avoids
  `next(ctx.lock)` boilerplate in multi-media and generic actions.
- **Single daemon-only `/api/x` with daemon PTY forwarding vs `/api/x` + `/api/x/resolve`**:
  Interactive tools (`embed`, `rename`, `lyrics`, `theme`) open `/dev/tty` and
  rely on `stdio = "inherit"`. Keeping resolution on the daemon (`/api/x/resolve`)
  and execution in the CLI process preserves native terminal behaviour without
  a PTY proxy.
- **Flat `actions` map with client filtering vs nested `ActionSet`**:
  `ActionSet { generic, media }` allows per-media `label(media)` and
  `description(media)` without reserving `"generic"` as a forbidden media name.
- **Per-media action declarations merging by name**: Merging `d.action`
  across files hid overwrites silently; requiring one declaration per canonical
  name keeps each action's full media scope in one place.

## Risks & Limits

- **Drive-by browser `POST /api/x` to `d.system`**: Closed by restricting
  `CorsLayer` to loopback + `192.168.0.0/16` + explicit `server.allowed_origins`
  and requiring `application/json` (forcing CORS preflight). Note: any trusted
  device within `192.168.0.0/16` (or `allowed_origins`) can invoke configured
  actions.
- **Daemon-spawned generic actions (`isatty = false`)**: Targetless actions
  invoked via HTTP (`open_config_in_terminal`) run in the daemon's environment
  and cwd; actions assuming a graphical display or terminal launcher require
  those environment variables on the daemon service unit.
- **Breaking existing action configs**: Any existing `actions.lua` reading
  `ctx.items` or string-based `ctx.options` must migrate before upgrading.

## Known consequences

- An action runs once per `(dir, media)` pair (or once with `ctx.lock = nil,
  ctx.media = nil` when targetless generic).
- The old selectors leave `x`; callers pass `--dir` and `--media` instead.
- `POST /api/actions/{name}` and `POST /api/select` are retired in favour of
  `POST /api/x` and `POST /api/x/resolve`.
- Existing user configs (e.g. `~/.config/datamile/actions.lua`) must migrate
  from `ctx.items` / string `ctx.options` to `ctx.lock[ctx.media]` / string-array
  `ctx.options`:
  ```lua
  -- before:
  local is_force = ctx.options:find("%-f") or ctx.options:find("%-%-force")
  for _, entry in ipairs(ctx.items) do ... entry.path ... end

  -- after:
  local is_force = d.list.contains(ctx.options, "-f") or d.list.contains(ctx.options, "--force")
  local path = ctx.lock[ctx.media].location.path
  ```

## Non-goals

- Changing the logic manifest (`filters`, `groupers`, `orders`, `libraries`,
  `shelves`, `cabinets`) to the `ActionSet` shape.
- Making the web app render entities of arbitrary non-album media types beyond
  passing the rendered entity's media type and `location.path`.

## Implementation

### Files

- `core/datamile-lua-domain/src/config.lua`: normalize `name` (`-` -> `_`),
  store without merging, reject duplicate canonical names, drop `config`.
- `core/datamile-lua-domain/src/core.lua`: `__DATAMILE_GET_ACTIONS` validates
  shape, skips label/description callbacks for unregistered media roots, returns
  `ActionSet`.
- `core/datamile-lua-domain/src/config.rs`: `ActionDef`, `ActionSet`, and
  `ServerConfig.allowed_origins` (`Vec<String>`).
- `core/datamile-lua-domain/src/engine.rs`: `EvaluatedDomainData.actions` and
  `ResolvedDomainConfig.actions` become `ActionSet`; `execute_action` calls
  `run(ctx)` with `{ lock, media, options, isatty }`.
- `core/datamile-media/src/schema.rs`: `validate_media_schema` takes
  `(&MediaManifest, &LogicManifest, &ActionSet)` and checks action media via
  `require_media` (updating callers in `media/backend.rs` and 7 `media/tests.rs`
  sites).
- `core/datamile-media/src/ingest.rs`: include `"location": entity.get("location")`
  in `dict` so `location.path` is available synchronously in `collection.dict`.
- `core/datamile-media/src/state.rs`: remove unused `MediaState.actions`.
- `core/datamile-media/src/backend.rs` and `watcher.rs`: update `ServerConfig`
  construction, remove `param_targets` / `playing_target` / `item_for` /
  `resolve_action_targets`, send `ActionSet` in `LOGIC_UPDATE`.
- `core/datamile-server/src/state.rs`, `backend.rs`, and `lib.rs`: `ActionSet` on
  `ServerConfig`, remove `ActionTargetQuery` and `resolve_action_targets`,
  re-export `ActionDef`/`ActionSet`, enforce `server.allowed_origins` CIDR/host
  CORS policy.
- `core/datamile-server/src/api/mod.rs`, `x.rs`, `websocket.rs`, `system.rs`:
  add `api/x.rs` (`/api/x` and `/api/x/resolve`), wire routes in `api/mod.rs`,
  delete `api/actions.rs` and `api/select.rs`, serialize `ActionSet` directly.
- `core/datamile/src/main.rs` and tests: narrow `XArgs` (`--dir`, `--media`,
  trailing `--` options), share target resolver via `/api/x/resolve` or offline
  `state.json`, execute locally; `intermediary` prints resolved `ctx` array.
- `interfaces/web-app/src/api.ts`, `types.ts`, `library/collection.svelte.ts`,
  `modules/HomeView/ModalDrawer/ModalDrawer.svelte`,
  `modules/NavigationBar.svelte`, `App.svelte`: `ActionSet` types, `location` on
  `AlbumDictEntry`, `action(name, mediaType)` lookup, `executeAction(name, media, dir, options?)`
  across all 8 `hasAction` call sites (`App.svelte` ×3, `ModalDrawer.svelte` ×4,
  `NavigationBar.svelte` ×1) and 5 label reads (`ModalDrawer.svelte` ×4,
  `NavigationBar.svelte` ×1).
- `docs/vocabulary.md`, `docs/lua-api.md` (§9, §10.1, §10.2, §11), and `README.md`.

### Phases

Each phase compiles and passes `devenv shell -- lint` and `devenv shell -- tests`
(with phases 1–4 landing together before release so server and web app stay in
sync on the wire):

1. **Lua & schema**: `ActionDef` / `ActionSet`, `d.action` normalization &
   collision check, `__DATAMILE_GET_ACTIONS`, `validate_media_schema` 3rd
   parameter, `ServerConfig.allowed_origins`.
2. **Routes & CORS**: `POST /api/x`, `POST /api/x/resolve`, shared target
   resolver, delete `actions.rs` / `select.rs` / `ActionTargetQuery`, CIDR CORS
   layer, `location` in `ingest.rs` `dict`.
3. **CLI**: narrowed `XArgs`, `/api/x/resolve` + offline `state.json` execution,
   `intermediary` JSON output, CLI integration tests.
4. **Web app & docs**: `ActionSet` store, `action(name, mediaType)`, updated
   call sites, `bun test` unit tests for lookup logic, `docs/` + `README.md`.

### Tests

- `datamile-lua-domain`: duplicate canonical name (`foo-bar` vs `foo_bar`)
  errors; `media = ""` and `media = {}` error; repeated media dedups; label
  function returning `nil` or `""` errors; label/description receive each
  declared media string; generic action lands in `generic`; legacy `config = {}`
  is ignored without error.
- `datamile-media`: unknown media in `d.action` (even with `label = function(...)`)
  fails cleanly in `validate_media_schema` with `"declares unknown media type"`;
  `ingest.rs` `dict` includes `location`.
- `datamile-server`: `/api/x` and `/api/x/resolve` return exact schemas and
  status codes (`200`, `400` on missing/partial target or invalid `--media`, `404`
  on unknown action or unindexed entity naming `datamile update`); targetless
  generic works; explicit `media` runs regardless of action's declared `media`;
  `INIT` / `LOGIC_UPDATE` carry `ActionSet`; `CorsLayer` allows `localhost`,
  `192.168.0.0/16` (any port), and configured `allowed_origins`, while rejecting
  public origins.
- `datamile`: CLI cross product order, Option B omission filtering, offline
  `state.json` parity, non-zero exit when any run fails while remaining runs
  execute, targetless generic runs with `ctx.lock = nil, ctx.media = nil,
  isatty = true`.
- `interfaces/web-app` (`bun test` + `bun run lint`): `action(name, type)`
  prefers `media[type]`, falls back to `generic`, and hides actions declared
  for other media types.

