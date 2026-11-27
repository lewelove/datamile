# Datamile Lua API & Media Engine Reference

> **Scope:** Full API reference for `d.*`, `d.media`, the media engine, the HTTP/WebSocket server, and the `datamile` CLI.  
> **Core Rule:** Rust owns performance, parallelism, caching, filesystem watching, indexing, HTTP/WS transport, and MPD control—with **zero** knowledge of specific media types (`album`, `track`, `movie`, `book`, etc.), aside from MPD's temporary `"album"` consumer exception. Lua defines what every media type is, how it is discovered, compiled, queried, and acted upon.

## 1. Global Namespaces & Crate Responsibilities

In Lua, `_G.d` and `_G.datamile` are interchangeable aliases of the exact same table (`_G.d == _G.datamile`).

| Layer | Crate | Responsibility |
| --- | --- | --- |
| Runtime + Stdlib | `datamile-lua` | `mlua::Lua` VM, unified dependency tracking (`require`, `dofile`, `loadfile`, `d.fs.*`), pure `d.*` stdlib (`fn`, `fs`, `get`, `iter`, `json`, `list`, `str`, `system`, `tbl`, `toml`, `fmt`), generic `init.lua` locator, per-entity reader memoization, compile-time `CompileGuard` blocking `*.lock.json` reads at `record_dependency`. |
| Domain API | `datamile-lua-domain` | `d.config`, `d.interface`, `d.action`, `d.library`, `d.filter`, `d.grouper`, `d.order`, `d.shelf`, `d.cabinet`, generic `logic.lua`, `ActionMap`, Datamile config path discovery. |
| Media Capabilities | `datamile-fs`, `datamile-cache`, `datamile-audio`, `datamile-images` | Pure atomic FS/path/hash utilities, disk object & `FileStat` caching, `EntityKey`, read-only audio probing, master QOI + derivative image rendering. |
| Media Engine | `datamile-media` | `d.media.root` / `d.media.node` registration, Phase 0 static validation, Phase 1 discovery & verification, Phase 2 parallel tree compiler, generic `inotify` watcher feeding `run_server_update`, generic lock ingestion, `MediaBackend` implementation. |
| Query Index | `datamile-logic` | Per-`media_type` `RoaringBitmap` index, `SortKey` sorting, flat grouper buckets, search, multi-owner playable & CAS image lookups. |
| Transport & Playback | `datamile-mpd`, `datamile-server` | Reconnecting MPD actor, raw status broadcast, Axum HTTP/WS server, `Host`/`Origin`/CORS enforcement, `ImageTaskRegistry`, generic debounced `notify` watcher loop. |

## 2. Configuration (`d.config`)

```lua
d.config({
  storage = {
    cache = "~/.cache/datamile",       -- default
    state = "~/.local/share/datamile", -- default
  },
  server = {
    host = "127.0.0.1",                -- default (set to "0.0.0.0" or LAN IP for LAN access)
    port = 8000,                       -- default
    allowed_origins = {                -- optional; extends built-in defaults
      "10.0.0.0/8",
      "box.local",
    },
  },
  mpd = {
    enabled = true,                    -- default
    host = "127.0.0.1",                -- optional; falls back to MPD_HOST env or 127.0.0.1
    port = 6600,                       -- optional; falls back to MPD_PORT env or 6600
    music_directory = "~/library",     -- must match MPD's; subtracted from every URI
  },
  compiler = {
    jobs = nil,                        -- optional usize; defaults to available parallelism
  },
})
```

### Rules
- `d.config` is closed: its only sections are `storage`, `server`, `mpd` and `compiler`. Any other
  key, at the top level or inside a section, is a typo and fails the config instead of being ignored.
- `server.host` and `server.port` are bound directly by `datamile server`. The
  host must be an IP address (`127.0.0.1`, `0.0.0.0`, `fd00::1`); a hostname
  fails config load.
- `server.allowed_origins` accepts hostnames, single addresses (`10.0.0.5`,
  `[fd00::1]`) and `ip/prefix` CIDR blocks. A port never belongs here: the
  guard strips the request port before it compares, so `box.lan:8080` fails
  config load.
  Built-in defaults always allow loopback (`localhost`, `127.0.0.1`, `[::1]`) and `192.168.0.0/16`;
  entries in `server.allowed_origins` extend those defaults. Malformed entries fail config load.
- Across all mutating `/api/*` routes, the server checks `Host` (matching loopback, the bound host,
  or `allowed_origins`) and `Origin` (rejecting `Origin: null` and origins outside the allowlist).
- `storage` contains **no** `music_directory`; each `d.media.root` owns its own `init` root directory.
- `mpd.music_directory` must equal MPD's own setting: a playable URI is the lock's absolute path
  (the entity's `location.path` joined with the node's own `location.path`) with that base
  subtracted. A file outside the base is sent as an absolute path, which MPD refuses and datamile
  logs — nothing guesses a different base.
- All `storage` and `mpd.music_directory` paths resolve relative to `config_dir` when relative, and expand `~`.

## 3. Standard Library (`d.*` / `datamile.*`)

All existing `d.*` modules (`fn`, `fs`, `get`, `iter`, `json`, `list`, `str`, `system`, `tbl`, `toml`) remain media-agnostic. Additions required for `d.media`:

### 3.1 `d.fs`
- Existing: `d.fs.normalize(path)`, `d.fs.read_toml(path)`, `d.fs.read_json(path)`, `d.fs.read(path)`, `d.fs.read_lines(path)`, `d.fs.read_dotenv(path)`, `d.fs.exists(path)`, etc.
- **Discovery helpers:**
  - `d.fs.roots(dir, marker, opts?)` — walks `dir` (following symlinks) and returns canonical directory paths containing the marker. `marker` is a marker path below `dir` (e.g., `"album.toml"` or `"Info/meta.json"`) or a flat array of names. Markers must be relative and may not contain `.` or `..` components. `opts.prune = true` stops the walk below a matched directory (default `false`). Bad `dir`, `marker` or `opts` raise an error.
  - `d.fs.files(dir, opts?)` — returns naturally-sorted (`alphanumeric-sort`) file entries under `dir` (`opts`: `{ ext = { ... } | "flac", recursive = true | false, paths_only = true | false }`). Each entry is stat-enriched in a single `readdir`/`walkdir` pass: `{ path, name, stem, ext, mtime, byte_size }` (or plain paths when `paths_only = true`), avoiding redundant `stat()` syscalls.
  - `d.fs.dirs(dir, opts?)` — naturally-sorted subdirectory entries under `dir`.
  - `d.fs.first(dir, candidates, opts?)` — returns the first matching existing file path from `candidates` (supporting exact filenames or extensions), or `nil`.
- **Audio reader (injected by `datamile-media`):**
  - `d.fs.read_audio(path)` — probes audio tags and stream physics via `datamile-audio`, backed by `<cache_root>/objects/` (`datamile-cache`). `datamile-lua` itself remains blind to `datamile-audio`.
- **Per-entity in-memory memoization:**
  - During compilation of a single entity, `d.fs.read_toml`, `d.fs.read_json`, and `d.fs.read_audio` memoize their Lua result in `EngineContext` keyed by canonical file path so repeated reads across entity/nodes (`tags`, `objects`, etc.) hit RAM once. The memo map is cleared immediately after the entity finishes compiling.
- **Compile-time lock-read ban (`CompileGuard`):**
  - While an entity is compiling, reading any `*.lock.json` file via `d.fs` fails at `EngineContext::record_dependency` as a config error, while directory enumerators (`files`, `dirs`, `roots`, `first`, `dir`) filter out `*.lock.json` entries. Raw Lua `io.open` is outside `d.fs` and is neither tracked nor guarded. Locks are outputs, never inputs.

### 3.2 `d.str`, `d.list`, `d.fmt`
- `d.str.leading_int(s)` — parses an integer prefix (e.g., `"01.flac"` → `1`, `"3/12"` → `3`, or `nil`).
- `d.list.sum(xs, fn?)` — sums numbers (or `fn(item)` over `xs`).
- `d.fmt.duration(ms)` / `d.fmt.bytes(n)` — pure display formatting (replaces Rust-side `format_ms`).

## 4. Media Schema DSL (`d.media.root` & `d.media.node`)

### 4.1 `d.media.root(name, def)`
Registers a top-level media type (e.g., `"album"`, `"movie"`, `"show"`, `"book"`).

```lua
local tracks = d.media.node("track", { ... })

local album = d.media.root("album", {
  init = "~/library",

  -- Called with `init` on full scan, or with a target directory during hot reload.
  -- Must return an array of directory paths (strings).
  select = function(dir)
    return d.fs.roots(dir, { "album.toml", "virtual.toml" })
  end,

  -- Optional body hook: runs after `path` and `location` are seeded.
  -- Populate `ctx` here so `.tag`, `.object`, `.image`, and child nodes pull from `ctx`.
  ctx = function(ctx)
    local meta = d.fs.read_toml(d.fs.joinpath(ctx.path, "album.toml")) or {}
    ctx.album = meta.album or {}
    ctx.entries = meta.tracks or {}
    ctx.files = d.fs.files(ctx.path, { ext = { "flac", "mp3" }, recursive = true, paths_only = true })
    ctx.cover = d.fs.first(ctx.path, { "cover.jpg", "cover.png" })
  end,

  -- Unique identifier within this media_type.
  id = function(ctx)
    return ctx.tags.albumartist .. " - " .. ctx.tags.album
  end,

  nodes = { "track" },
})

album.tag("album", function(ctx) return ctx.album.album end)
album.tag("albumartist", function(ctx) return ctx.album.albumartist end)
album.object("info", function(ctx) ... end)
album.image("main", function(ctx)
  return {
    fit = { "crop", 1080, 1080 },
    filter = "lanczos",
    sources = ctx.cover,
  }
end)
```

#### Entity Invariants (Enforced by Rust)
1. **Per-type directory uniqueness & nesting:** Within one `media_type`, `select` cannot select duplicate dirs or nested dirs (one entity dir inside another). Across *different* `media_type`s, shared directories (`album.lock.json` and `vinyl.lock.json` in one dir) and cross-type nesting are allowed.
2. **Entity `location`:** Every entity always emits `location = { kind = "dir", path = "<canonical abs dir>" }`.
3. **Output lock file:** Written atomically inside the entity dir as `<name>.lock.json` (e.g., `album.lock.json`, `movie.lock.json`). Leftover locks from removed entities are never deleted or renamed.
4. **Unique `(media_type, id)`:** Duplicate `id`s within the same `media_type` fail with a clear error naming both conflicting directories. Two *different* `media_type`s may share the same `id` without collision.

### 4.2 `d.media.node(name, def)`
Defines a child node referenced by literal name in `root.nodes` or nested inside another `node.nodes` (arbitrary depth: e.g., `show -> season -> episode`, `album -> track`).

```lua
local track = d.media.node("track", {
  -- Receives parent `ctx` (`path`, `location`, `tags`, `objects`, `parent`, plus user fields).
  -- Return array of string paths (physical nodes -> `location` attached)
  -- OR array of tables (virtual/logical nodes -> NO `location` attached).
  select = function(ctx) return ctx.files end,

  -- Optional body hook: prepare per-node data on `ctx` once for downstream hooks.
  ctx = function(ctx)
    ctx.audio = d.fs.read_audio(ctx.path)
  end,
})

track.tag("title", function(ctx) ... end)
track.object("info", function(ctx) ... end)
```

#### Node `select` Return Contract
- **String path (`"01.flac"` or `"/abs/path/01.flac"`):**
  - Rust resolves relative to the entity dir, stats the target, and attaches `location`:
    - File: `{ kind = "file", path = "<rel_to_root>", mtime = <u64_secs>, byte_size = <u64> }`
    - Directory: `{ kind = "dir", path = "<rel_to_root>" }` (no stats; a dir's mtime moves when the lock is written)
  - `ctx.location` and `ctx.path` (resolved path) are passed into `ctx(ctx)`, `tag(ctx)`, `object(ctx)`, `image(ctx)`, and nested `nodes`.
- **Table (`{ title = "Intro", ... }`):**
  - Virtual / logical node: **`location` is omitted**.
  - `ctx.item` (the returned table) is passed alongside `ctx.parent` into `ctx(ctx)`, `tag(ctx)`, `object(ctx)`, `image(ctx)`, and nested `nodes`.

### 4.3 Evaluation Context (`ctx`) & Dependency Order
`ctx` is built hybridly—auto-seeded by the compiler, optionally enriched by your `ctx(ctx)` hook, and progressively filled as phases run (one Rust↔Lua boundary crossing per entity):
1. **Auto-seeded fields:** Root entities start with `{ path, location }`; child nodes start with `{ parent = parent_ctx, item?, path, location? }`.
2. **Manual `ctx(ctx)` hook (optional):** Mutates `ctx` in place before any `.tag`, `.object`, `id`, `.image`, or child `nodes` run.
   - **Recommended pattern:** Run directory scans (`d.fs.files`, `d.fs.first`), shared manifest reads, and shared derivations once inside `ctx(ctx)` and pull from `ctx` in `.tag` / `.object` / `.image`. While `d.fs.read_toml`, `d.fs.read_json`, and `d.fs.read_audio` are memoized per entity, `d.fs.files`, `d.fs.first`, `d.fs.dirs`, `d.fs.read`, and Lua derivations are not; only call `d.fs.read` directly inside a hook when that single hook consumes the file.
3. **Registered `.tag(name, fn)` hooks:** Run against `ctx` and populate `ctx.tags` (`.tag` hooks run before `ctx.objects` and `ctx.id` exist).
4. **Registered `.object(name, fn)` hooks:** Wrapped in a lazy `ctx.objects` proxy (accessing `ctx.objects.other` evaluates `objects.other(ctx)` once and caches it; cycles raise an immediate error). `.object` hooks can read `ctx.tags` and other `ctx.objects`.
5. **`id(ctx)` (root entity only):** Runs with `{ path, location, tags, objects, ... }` and sets `ctx.id`.
6. **Registered `.image(name, fn)` hooks:** Run with `{ path, location, tags, objects, id?, ... }`. Returning `nil` omits that namespace; returning `{ fit, filter, sources }` delegates to `datamile-images` and emits `{ address, width, height, fit, filter, source = { file = { path, mtime, byte_size } } }` (no `hash`).
7. **Child `nodes`:** Evaluates each child node recursively with `ctx.parent = current_ctx`, then materializes any remaining lazy objects.
8. **Serialization:** Empty tables/nulls/empty strings are stripped, object keys sorted deterministically, and the finished entity is converted once from Lua to JSON and wrapped under `{ "<media_type>": { ... } }`.

## 5. Compiled Lock JSON Format (`<type>.lock.json`)

Literal JSON on disk (and returned verbatim by `GET /api/media/{type}/{*id}`):

```json
{
  "album": {
    "id": "Artist - Album",
    "location": { "kind": "dir", "path": "/library/Artist - Album" },
    "tags": { "album": "Album", "albumartist": "Artist", "date": 2024 },
    "objects": {
      "info": { "total_discs": 1, "total_tracks": 2, "duration_milliseconds": 360000 }
    },
    "images": {
      "main": [
        {
          "address": "abc123...",
          "width": 1080,
          "height": 1080,
          "fit": "crop",
          "filter": "lanczos",
          "source": {
            "file": { "path": "cover.jpg", "mtime": 1700000000, "byte_size": 1048576 }
          }
        }
      ]
    },
    "nodes": {
      "track": [
        {
          "location": { "kind": "file", "path": "01.flac", "mtime": 1700000000, "byte_size": 31000000 },
          "tags": { "title": "Opening", "tracknumber": 1, "discnumber": 1 },
          "objects": { "info": { "duration_milliseconds": 180000, "duration_formatted": "3:00" } }
        },
        {
          "tags": { "title": "Virtual Bonus Track", "tracknumber": 2, "discnumber": 1 }
        }
      ]
    }
  }
}
```

- Top-level key is always the `<media_type>` name (`"album"`, `"movie"`, `"show"`, etc.).
- Empty `nodes`, `images`, `objects`, `tags` are omitted via empty-value stripping.
- Virtual nodes omit `location` completely.

## 6. Execution Pipeline (Phases 0, 1, 2)

1. **Phase 0 — Static Validation (at config load, before touching `init` directories):**
   - Validates all `d.media.root` definitions (`init` path present, `select`/`id` functions present, valid `nodes` definitions).
   - Restricts every media type name to `[a-z0-9_-]+`, so the name is safe as a path segment under the cache.
   - Validates every `library`, `filter`, `grouper`, `order`, `shelf`, `cabinet`, and `action` media declaration against registered `media` types via `require_media`, then evaluates per-media `label`/`description` callbacks on `d.action`.
   - Validates `mpd`, `storage`, and `server` (`host`, `port`, `allowed_origins`) settings.
2. **Phase 1 — Discovery & Incremental Verification:**
   - Calls `root.select(init)` per `d.media.root`, canonicalizes & dedupes entity dirs per `media_type`, and rejects same-type nested entity dirs (cross-type dir sharing and nesting are allowed).
   - Loads one state file per media root: `<cache>/configs/<hash(config path)>/roots/<media_type>/<hash(init)>/state.json`. Each `RootState` entry records input `files` (excluding `*.lock.json`) plus `lock_stat: Option<FileStat>` of `<dir>/<media_type>.lock.json`.
   - An entity is dirty (`root_is_dirty`) when it has no entry, `lock_stat` is `None`, the walked or external files changed/vanished, `<media_type>.lock.json` is missing on disk, or its on-disk `FileStat` differs from `lock_stat`. Removed entities are `state.keys() - discovery`. A config change never marks an entity dirty: rebuild the locks with `datamile update --force` after editing compile hooks.
3. **Phase 2 — Parallel Compilation (`rayon`, 1 Lua VM per worker thread):**
   - Each worker thread initializes one `DomainLuaEngine`, activates `CompileGuard` (`EngineContext`) to forbid reading any `*.lock.json` via `d.fs`, and clears dependencies and per-entity memoization before/after each entity.
   - Writes `<media_type>.lock.json` atomically only when the serialized JSON differs from disk (failing only that entity on `PermissionDenied`/`EROFS` read-only paths). Records `lock_stat` in `RootState` for **every** compiled entity, written or unchanged.

## 7. Hot Reloading & Single Write Path (`datamile-media` + `datamile-server`)

Hot recompilation is media-agnostic and routes through one serialized pipeline (`run_server_update`):

1. **Watch Set:**
   - `config_dir` (recursive) + non-recursive parent dirs of `config_deps` (external Lua files `require`d / read by `init.lua`).
   - Every `d.media.root`'s canonical `init` directory (recursive).
   - Non-recursive parent dirs of the external files listed in the state files. The `ExternalIndex` (`BTreeMap<PathBuf, BTreeSet<EntityKey>>`) is rebuilt from those state files at boot; nothing on disk duplicates it.
2. **Self-Write Suppression (`active_writes`):**
   - Any `<media_type>.lock.json` written by `datamile-media` is registered in `active_writes` as a latency optimization so self-triggered `inotify` events are skipped. Matching `lock_stat` ensures termination regardless.
3. **Config Dependency Changes:**
   - Any event matching `config_deps` reloads `init.lua` + `LogicManifest` + `ActionMap` + `ServerConfig`, re-evaluates all indexed entities against the new manifest, and broadcasts `LOGIC_UPDATE { manifest, actions }`. It never recompiles locks: most config edits (actions, interfaces, logic) do not change compiled output, and the rest are rebuilt by `datamile update --force`. A reload that changes `server.port` or `server.host` logs it and exits so the next start binds the new address. A new `server.allowed_origins` takes effect at once: the request guard and the CORS policy read one allowlist that the reload replaces in place. A failed reload is remembered and reported by `GET /api/identity`.
4. **Media & External Dependency Events → `run_server_update`:**
   - The watcher does not compile or write locks/state on its own, and holds no `logic`, `state_writer`, or `update_lock` locks across submits. Instead, it resolves affected targets and submits them into `run_server_update`:
     - External dependency hits read `ExternalIndex` directly into a `BTreeSet<EntityKey>`.
     - Watch-root events resolve affected `(media_type, init)` pairs via `known_root_ancestor` and `select_roots` into `EntityKey`s (or scoped `Paths` when an ancestor directory disappears).
   - `run_server_update` acquires locks in fixed order (`update_lock` -> `state_writer` inside `run_pass` -> `logic.write()`), compiles dirty targets, records `lock_stat` and `files` for every compiled entity, updates `LogicEngine` and `ExternalIndex`, and broadcasts `MEDIA_UPDATED` only for entities whose lock changed or that were added/removed.

## 8. Logic DSL (`library`, `filter`, `grouper`, `order`, `shelf`, `cabinet`)

Every logic unit **must** declare its single `media = "<root_name>"` type. Media types are **never mixed** in libraries, filters, groupers, orders, shelves, or cabinets.

```lua
d.library("library", {
  media = "album",
  label = "Music Library",
  match = function(album) return true end,
})

d.filter("owned", {
  media = "album",
  label = "Owned",
  match = function(album) return album.objects.info.owned end,
})

-- Groupers have NO `format` callback. `select(entity)` returns a string or string[].
d.grouper("genre", {
  media = "album",
  label = "Genres",
  select = function(album) return album.tags.genre end,
})

d.order("alpha", {
  media = "album",
  label = "Title",
  reverse = false,
  sort = function(album) return album.tags.album end,
})

d.shelf("recent", {
  media = "album",
  label = "Recently Added",
  reverse = true,
  match = function(album) return true end,
  sort = function(album) return album.tags.date end,
})

d.cabinet("default", {
  media = "album",
  label = "Music",
  shelves = { "recent" },
  orders = { "alpha" },
})
```

### Logic Engine Behavior (`datamile-logic`)
- **Partitioned per `media_type`:** Each registered `media` root has its own isolated namespace (`(media_type, id)`), bitmaps, orders, shelves, and groupers.
- **Flat Groupers:** `grouper` builds `HashMap<String, RoaringBitmap>` buckets directly from `select(entity)`, sorted via `SortKey` on the key string (`reverse = true | false`). Output nodes are `{ value, label, count, children = {} }` (`sublabel` is omitted). Zero Lua calls happen during `build_cache` or `GROUP_REQUEST`.
- **Generic Search:** `search` matches against the entity's `id` plus the entity's indexed tag strings.
- **Multi-owner Playable & CAS Image Lookups:**
  - `playable_lookup` maps each URI (`path`) to `Vec<PlayableLookupItem>` (`{ media_type, media_id, node_index, node_path, path, meta }`), retaining all owners when multiple media types share one file. As a temporary exception until MPD is retired in favour of `datamile-mpv`, `mpd_bridge` queries `playable_lookup` for `"album"`.
  - `image_lookup` maps each CAS `address` to `{ spec: { fit, filter, target_width, target_height }, sources: Vec<(EntityKey, PathBuf)> }`. When `<cache_root>/images/<address>/master.qoi` is missing from disk, the server tries `sources` in order until an existing file is found.

## 9. Actions & Interfaces (`d.action`, `d.interface`)

```lua
d.action("open_terminal", {
  media = { "album", "movie" },
  label = function(media)
    return media == "album" and "Open Album Terminal" or "Open Movie Terminal"
  end,
  description = "Open terminal in entity directory",
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

d.interface("default", {
  label = "Web UI",
  run = function(ctx)
    -- ctx = { server = { host, port, url, ws_url } }
  end,
})
```

- **No built-in actions.** Datamile ships none; every action is declared by the user in `init.lua`.
- **Canonical action names:** `d.action` normalizes `-` -> `_` at declaration time and errors on duplicate canonical names.
- **`media` scope (`nil | string | string[]`):** `media = nil` is generic; otherwise scoped to the listed media types. `media` drives load-time validation, client UI `ActionMap` generation (`{ label, description, media?: { [type]: { label, description } } }`), and default media selection when `--media` is omitted. Explicit `--media` selects `<dir>/<media>.lock.json` literally for any registered media type.
- **`run(ctx)` context:** `ctx = { lock, media, options, isatty }`, where `ctx.lock` is the parsed `<dir>/<media>.lock.json` JSON (`nil` when targetless), `ctx.media` is the resolved media type (`nil` when targetless), `ctx.options` is the trailing `--` argument array, and `ctx.isatty` is `true` when CLI stdin is a terminal (`false` on daemon-executed HTTP runs).

## 10. WebSocket & HTTP API Contract

### 10.1 WebSocket (`/ws`)
**Server → Client:**
- `INIT`: `{ type = "INIT", media = { "<type>": { "<id>": SurfaceItem } }, manifest, actions: ActionMap, shelves, ui_state }`
- `MEDIA_UPDATED`: `{ type = "MEDIA_UPDATED", media_type, updated: { "<id>": SurfaceItem }, removed: ["<id>"], shelves }`
- `LOGIC_UPDATE`: `{ type = "LOGIC_UPDATE", manifest, actions: ActionMap }`
- `VIEW_DATA`: `{ type = "VIEW_DATA", media_type, ids: ["<id>"] }`
- `SHELF_DATA`: `{ type = "SHELF_DATA", media_type, ids: ["<id>"] }`
- `GROUP_RESULT`: `{ type = "GROUP_RESULT", media_type, key, result: [{ value, label, count, children }] }`
- `MPD_STATUS`: `{ type = "MPD_STATUS", state, media_type, media_id, node_index, file, elapsed, duration, title, artist, queue }`

**Client → Server:**
- `VIEW_REQUEST`: `{ type = "VIEW_REQUEST", media_type, library, library_filter, sort, reverse, filter }`
- `SHELF_REQUEST`: `{ type = "SHELF_REQUEST", media_type?, shelf, order, reverse }`
- `GROUP_REQUEST`: `{ type = "GROUP_REQUEST", media_type, library, library_filter, key }`

### 10.2 HTTP (`/api/...`)
- `GET /api/media` → `{ types: { "<type>": count } }`
- `GET /api/media/{type}?q=...` → `[{ type, id, path }]`
- `GET /api/media/{type}/{*id}?depth=<n>` → Literal `<type>.lock.json` from disk, optionally truncated to `depth` levels of `nodes`.
- `POST /api/media/{type}/update/{*id}` → Forces recompile of one `(type, id)` entity.
- `POST /api/media/{type}/play/{*id}?start=&count=&offset=` → Plays playable slice in MPD.
- `POST /api/media/{type}/queue/{*id}?start=&count=` → Appends playable slice to MPD queue.
- `POST /api/playback/{jump/{index},next,prev,stop,clear,toggle-pause}`
- `POST /api/x` → Unified action endpoint (`{ name, dirs?, media?, options?, execute? }`, `execute` defaults to `true`; returns `{ runs: [{ media, dir, lock?, error? }] }`). Enforces `application/json`, `Host`, and `Origin`/`CorsLayer` policy; `execute: false` returns resolved lock targets for CLI execution and rejects browser `Origin`/`Sec-Fetch-Site` headers.
- `GET /api/images/{file}?size=&filter=&quality=` & `POST /api/images/cache`
- `POST /api/state`, `POST /api/update` (`{ path?, media_type?, entities?, force, jobs? }`, NDJSON event stream), `GET /api/identity`, `POST /api/reset`

## 11. CLI Contract (`datamile`)

Global flags: `--config <dir>`, `-v` (log filter to debug, which is what shows `Unchanged`), `--silent` (print nothing).

- `datamile server` — runs `datamile-server` backed by `datamile-media`, binding `server.host` and `server.port` from config. One daemon per user, enforced by a kernel lock at `~/.local/state/datamile/instance.lock` (`$HOME` only; `XDG_STATE_HOME` is ignored on purpose).
- `datamile compile <path> [--type <media_type>] [--pretty]` — renders the lock and prints it. It never writes a lock file and takes no instance lock; rendered images still land in the content-keyed cache.
- `datamile update [<path>] [--type <media_type>] [--force] [--jobs <n>]` — full discovery plus incremental compilation. A config change alone never makes an entity dirty, so use `--force` after editing compile hooks. A free instance lock means no daemon: the pass runs here holding the lock. A held lock means the configured port is asked; the daemon's `config_path`, version and reload state must match, or the command fails instead of writing locally. Any failure exits non-zero, including under `--silent`.
- `datamile x <action> [--dir <entity-dir>...] [--media <media_type>...] [-- <args>...]` — requires a matching running daemon (`InstanceLock` + `/api/identity`). Canonicalizes `--dir` locally (omitted `--dir` sends `[]` for generic actions or `[cwd]` for scoped actions), sends `POST /api/x` with `execute: false`, and invokes `run(ctx)` locally in the CLI process for each resolved run. Exits non-zero if the daemon is offline or any run fails.
- `datamile interface [<name>]`
