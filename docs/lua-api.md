# Datamile Lua API & Media Engine Specification

> **Status:** Authoritative target specification for `d.media` and all media-agnostic crates.  
> **Core Rule:** Rust owns performance, parallelism, caching, filesystem watching, indexing, HTTP/WS transport, and MPD control—with **zero** knowledge of specific media types (`album`, `track`, `movie`, `book`, etc.). Lua defines what every media type is, how it is discovered, compiled, queried, and acted upon.

---

## 1. Global Namespaces & Crate Responsibilities

In Lua, `_G.d` and `_G.datamile` are interchangeable aliases of the exact same table (`_G.d == _G.datamile`).

| Layer | Crate | Responsibility |
| --- | --- | --- |
| Runtime + Stdlib | `datamile-lua` | `mlua::Lua` VM, unified dependency tracking (`require`, `dofile`, `loadfile`, `d.fs.*`), pure `d.*` stdlib (`fn`, `fs`, `get`, `iter`, `json`, `list`, `str`, `system`, `tbl`, `toml`, `fmt`), generic `init.lua` locator, per-entity reader memoization hook. |
| Domain API | `datamile-lua-domain` | `d.config`, `d.interface`, `d.action`, `d.library`, `d.filter`, `d.grouper`, `d.order`, `d.shelf`, `d.cabinet`, generic `logic.lua`, Datamile config path discovery. |
| Media Capabilities | `datamile-fs`, `datamile-cache`, `datamile-audio`, `datamile-images` | Pure atomic FS/path/hash utilities, disk object & stat caching, read-only audio probing, master QOI + derivative image rendering. |
| Media Engine | `datamile-media` | `d.media.root` / `d.media.node` registration, Phase 0 static validation, Phase 1 discovery & verification, Phase 2 parallel tree compiler, generic `inotify` hot-reloader, generic lock ingestion, `MediaBackend` implementation. |
| Query Index | `datamile-logic` | Per-`media_type` `RoaringBitmap` index, `SortKey` sorting, flat grouper buckets, search & playable-node lookup. |
| Transport & Playback | `datamile-mpd`, `datamile-server` | Reconnecting MPD actor, raw status broadcast, Axum HTTP/WS server, `ImageTaskRegistry`, generic debounced `notify` watcher loop. |

---

## 2. Configuration (`d.config`)

```lua
d.config({
  storage = {
    cache = "~/.cache/datamile",       -- default
    state = "~/.local/share/datamile", -- default
  },
  server = {
    host = "127.0.0.1",                -- default
    port = 8000,                       -- default
  },
  mpd = {
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
- `storage` contains **no** `music_directory`; each `d.media.root` owns its own `init` root directory.
- `mpd.music_directory` must equal MPD's own setting: a playable URI is the lock's absolute path
  (the entity's `location.path` joined with the node's own `location.path`) with that base
  subtracted. A file outside the base is sent as an absolute path, which MPD refuses and datamile
  logs — nothing guesses a different base.
  - All `storage` and `mpd.music_directory` paths resolve relative to `config_dir` when relative, and expand `~`.

---

## 3. Standard Library (`d.*` / `datamile.*`)

All existing `d.*` modules (`fn`, `fs`, `get`, `iter`, `json`, `list`, `str`, `system`, `tbl`, `toml`) remain media-agnostic. Additions required for `d.media`:

### 3.1 `d.fs`
- Existing: `d.fs.normalize(path)`, `d.fs.read_toml(path)`, `d.fs.read_json(path)`, `d.fs.stat(path)`, etc.
- **Discovery helpers:**
  - `d.fs.roots(dir, marker_or_opts)` — walks `dir` (following symlinks) and returns canonical directory paths containing the marker (e.g., `"album.toml"`, or `{ markers = { "album.toml", "virtual.toml" } }`).
  - `d.fs.files(dir, opts?)` — returns naturally-sorted (`alphanumeric-sort`) file entries under `dir` (`opts`: `{ ext = { ... } | "flac", recursive = true | false }`). Each entry is stat-enriched in a single `readdir`/`walkdir` pass: `{ path, name, ext, mtime, byte_size }` (or plain paths when `paths_only = true`), avoiding redundant `stat()` syscalls.
  - `d.fs.dirs(dir, opts?)` — naturally-sorted subdirectory entries under `dir`.
  - `d.fs.first(dir, candidates, opts?)` — returns the first matching existing file path from `candidates` (supporting exact filenames or extensions/globs), or `nil`.
- **Audio reader (injected by `datamile-media`):**
  - `d.fs.read_audio(path)` — probes audio tags and stream physics via `datamile-audio`, backed by `<cache_root>/objects/` (`datamile-cache`). `datamile-lua` itself remains blind to `datamile-audio`.
- **Per-entity in-memory memoization:**
  - During compilation of a single root entity, `d.fs.read_toml`, `d.fs.read_json`, and `d.fs.read_audio` memoize their Lua result in `EngineContext` keyed by canonical file path so repeated reads across `root`/`nodes` (`tags`, `objects`, etc.) hit RAM once. The memo map is cleared immediately after the root finishes compiling.

### 3.2 `d.str`, `d.list`, `d.fmt`
- `d.str.leading_int(s)` — parses an integer prefix (e.g., `"01.flac"` → `1`, `"3/12"` → `3`, or `nil`).
- `d.list.sum(xs, fn?)` — sums numbers (or `fn(item)` over `xs`).
- `d.fmt.duration(ms)` / `d.fmt.bytes(n)` — pure display formatting (replaces Rust-side `format_ms`).

---

## 4. Media Schema DSL (`d.media.root` & `d.media.node`)

### 4.1 `d.media.root(name, def)`
Registers a top-level media type (e.g., `"album"`, `"movie"`, `"show"`, `"book"`).

```lua
d.media.root("album", {
  init = "~/library",

  -- Called with `init` on full scan, or with a target directory during hot reload.
  -- Must return an array of directory paths (strings).
  select = function(dir)
    return d.fs.roots(dir, { markers = { "album.toml", "virtual.toml" } })
  end,

  -- Unique identifier within this media_type.
  id = function(ctx)
    return ctx.tags.albumartist .. " - " .. ctx.tags.album
  end,

  -- Optional tag keys (or fn(entity) -> string[]) indexed for text search.
  -- Defaults to all scalar/array strings in root `tags`.
  search = { "album", "albumartist" },

  tags = function(ctx) ... end,

  objects = {
    info = function(ctx) ... end,
  },

  images = {
    main = function(ctx)
      return {
        fit = { "crop", 1080, 1080 },
        filter = "lanczos",
        sources = { d.fs.first(ctx.path, { "cover.jpg", "cover.png" }) },
      }
    end,
  },

  nodes = {
    track = d.media.node({ ... }),
  },
})
```

#### Root Invariants (Enforced by Rust)
1. **Unique canonical directories:** Every discovered root path is canonicalized and deduplicated in Rust. Two entries within the same root type or across *different* `d.media.root` types cannot claim the same canonical directory.
2. **Root `location`:** Every root always emits `location = { kind = "dir", path = "<canonical abs dir>" }`.
3. **Output lock file:** Written atomically inside the root directory as `<name>.lock.json` (e.g., `album.lock.json`, `movie.lock.json`).
4. **Unique `(media_type, id)`:** Duplicate `id`s within the same `media_type` fail with a clear error naming both conflicting directories. Two *different* `media_type`s may share the same `id` without collision.

### 4.2 `d.media.node(def)`
Defines a child node inside `root.nodes` or nested inside another `node.nodes` (arbitrary depth: e.g., `show -> season -> episode`, `album -> track`).

```lua
d.media.node({
  -- Receives parent `ctx` (`path`, `location`, `tags`, `objects`, `parent`).
  -- Return array of string paths (physical nodes -> `location` attached)
  -- OR array of tables (virtual/logical nodes -> NO `location` attached).
  select = function(ctx) ... end,

  tags = function(ctx) ... end,

  objects = {
    info = function(ctx) ... end,
  },

  images = { ... },

  nodes = { ... }, -- optional recursive child nodes
})
```

#### Node `select` Return Contract
- **String path (`"01.flac"` or `"/abs/path/01.flac"`):**
  - Rust resolves relative to the root directory, stats the target, and attaches `location`:
    - File: `{ kind = "file", path = "<rel_to_root>", mtime = <u64_secs>, byte_size = <u64> }`
    - Directory: `{ kind = "dir", path = "<rel_to_root>" }` (no stats; a dir's mtime moves when the lock is written)
  - `ctx.location` and `ctx.path` (resolved path) are passed into `tags(ctx)`, `objects(ctx)`, `images(ctx)`, and nested `nodes`.
- **Table (`{ title = "Intro", ... }`):**
  - Virtual / logical node: **`location` is omitted**.
  - `ctx.item` (the returned table) is passed alongside `ctx.parent` into `tags(ctx)`, `objects(ctx)`, `images(ctx)`, and nested `nodes`.

### 4.3 Evaluation Context (`ctx`) & Dependency Order
Inside one root compilation, evaluation proceeds in Lua tables (one Rust↔Lua boundary crossing per root):
1. `location` (and `path` for physical roots/nodes) is populated first.
2. `tags(ctx)` runs with `{ path, location, item?, parent? }`.
3. `objects` entries run lazily/memoized on demand via `ctx.objects`:
   - `ctx` exposes `tags` and `objects` (accessing `ctx.objects.other` evaluates `objects.other(ctx)` once and caches it; cycles raise an immediate error).
4. `id(ctx)` (on root) runs with `{ path, location, tags, objects }`.
5. `images` functions run with `{ path, location, tags, objects, id? }`. Returning `nil` omits that namespace; returning `{ fit, filter, sources }` delegates to `datamile-images` and emits `{ address, width, height, fit, filter, source = { file = { path, mtime, byte_size } } }` (no `hash`).
6. `nodes` evaluates each child `node` recursively with `ctx.parent = current_ctx`.
7. Empty tables/nulls/empty strings are stripped, object keys sorted deterministically, and the finished root is converted once from Lua to JSON and wrapped under `{ "<media_type>": { ... } }`.

---

## 5. Compiled Lock JSON Format (`<type>.lock.json`)

WYSIWYG JSON on disk (and returned verbatim by `GET /api/media/{type}/{*id}`):

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

---

## 6. Execution Pipeline (Phases 0, 1, 2)

1. **Phase 0 — Static Validation (at config load, before touching `init` directories):**
   - Validates all `d.media.root` definitions (`init` path present, `select`/`id`/`tags` functions present, no two roots sharing the same canonical `init` path if conflicting, valid `nodes` definitions).
   - Restricts every media type name to `[a-z0-9_-]+`, so the name is safe as a path segment under the cache (breaking change).
   - Validates every `library`, `filter`, `grouper`, `order`, `shelf`, `cabinet` declares a valid registered `media` type.
   - Validates `mpd` and `storage` settings.
2. **Phase 1 — Discovery & Incremental Verification:**
   - Calls `root.select(init)` per `d.media.root`, canonicalizes & dedupes root directories, checks global root-directory uniqueness across all `media_type`s.
   - Rejects nested root entities, in one media type or across two, so every file belongs to at most one of them (breaking change).
   - Loads one state file per media root, `<cache>/configs/<hash(config path)>/roots/<media_type>/<hash(init)>/state.json`, which lists the root entities that existed at the end of the last pass and the files each one read. A root entity is dirty when it has no entry, when the walked set changed, or when an external entry is missing or changed. Removed is that key set minus discovery. A config digest under the same config directory forces a full pass.
3. **Phase 2 — Parallel Compilation (`rayon`, 1 Lua VM per worker thread):**
   - Each worker thread initializes one `DomainLuaEngine` (`map_init`), clearing `EngineContext` dependencies and per-entity reader memoization before/after each root.
   - Writes `<type>.lock.json` atomically only when the serialized JSON differs from the existing file on disk.

---

## 7. Hot Reloading & Rust-Side Guards (`datamile-media` + `datamile-server`)

Hot recompilation must be seamless, media-agnostic, and free of false triggers or missed updates. All guards live in Rust:

1. **Watch Set:**
   - `config_dir` (recursive) + non-recursive parent dirs of `config_deps` (external Lua files `require`d / read by `init.lua`).
   - Every `d.media.root`'s canonical `init` directory (recursive).
   - Non-recursive parent dirs of the external files listed in the state files (e.g., cover art or metadata outside `init`). The reverse index `external path -> root entity` is derived from those state files at boot; nothing on disk duplicates it.
2. **Self-Write Suppression (`active_writes`):**
   - Any `<type>.lock.json` written by `datamile-media` is registered in `active_writes` so the resulting `inotify` event is ignored and never causes a recompile loop.
   - External edits/deletions of `<type>.lock.json` *not* in `active_writes` are treated as changes to that root.
3. **Config Dependency Changes:**
   - Any event matching `config_deps` reloads `init.lua` + `LogicManifest` + `ServerConfig`, re-evaluates all indexed entities against the new manifest, and broadcasts `LOGIC_UPDATE { manifest, actions }`. A reload that changes `server.port` logs it and exits: a bound port cannot change in place, so the service manager or the next command starts the daemon on the new port. A failed reload is remembered and reported by `GET /api/identity`, so no CLI delegates to a daemon with a stale config.
4. **External Dependency Changes:**
   - Any event matching an external entry in a state file resolves all dependent `(media_type, root_dir)` pairs and queues them for recompile + lock diff.
5. **Media `init` Subtree Events (`select(init)` + Ancestor Guard):**
   For each changed path `p` falling under a `d.media.root`'s canonical `init`:
   - **Case A — Inside a known root (`p` has an ancestor `R` registered in `LogicEngine` for that `media_type`):**
     - Works at any nesting depth (e.g. `Album/Disc 1/01.flac` or `Show/Season 1/E01.mkv` both resolve to `R`).
     - If `!R.exists()` (entire root deleted/moved away): drop `R`'s state entry through the same serialised writer the pass uses, remove `R` from `LogicEngine`, broadcast `MEDIA_UPDATED { media_type, removed: [id] }`. Its `<type>.lock.json` is never deleted.
     - If `R.exists()`: call `root.select(R)`.
       - If `select(R)` no longer contains `R` (e.g., its marker file `album.toml` was deleted): drop the state entry, unindex `R`, broadcast `MEDIA_UPDATED { media_type, removed: [id] }`. The lock stays.
       - If `select(R)` contains `R`: recompile `R`, compare serialized `<type>.lock.json` bytes with disk/current lock. **Only if bytes differ**: write lock atomically, re-ingest into `LogicEngine`, record the state entry, and broadcast `MEDIA_UPDATED`.
   - **Case B — Not inside any known root (new folder/subtree added, or `init` itself changed):**
     - If `!p.exists()`, check whether any known roots were prefixes under `p` (`record.path.starts_with(p)`) and remove any whose directory no longer exists.
     - Otherwise, pick the nearest existing ancestor directory `scan_dir` at or below `p` (clamped inside `init`) and call `root.select(scan_dir)`:
       - Canonicalize & dedupe returned root paths against all `d.media.root` types.
       - Skip already-known roots whose `FileStat` + deps are unchanged (unless directly targeted), compile new/dirty roots, diff locks, ingest, and broadcast `MEDIA_UPDATED`.

---

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
  select = function(album) return album.objects.info.owned end,
})

-- Groupers have NO `format` callback. `select(entity)` returns a string or string[].
d.grouper("genre", {
  media = "album",
  label = "Genres",
  order = "asc",
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
  libraries = { "library" },
})
```

### Logic Engine Behavior (`datamile-logic`)
- **Partitioned per `media_type`:** Each registered `media` root has its own isolated namespace (`(media_type, id)`), bitmaps, orders, shelves, and groupers.
- **Flat Groupers:** `grouper` builds `HashMap<String, RoaringBitmap>` buckets directly from `select(entity)`, sorted via `SortKey` on the key string (`order = "asc" | "desc"`). Output nodes are `{ value, label, count, children = {} }` (`sublabel` is omitted). Zero Lua calls happen during `build_cache` or `GROUP_REQUEST`.
- **Generic Search:** `search` matches against the entity's `id` plus the root's indexed `search` strings (extracted from configured `search` tag keys or all string values in root `tags`).
- **Generic Playable Lookup:** Built by walking every node with `location.kind == "file"`, recording `{ media_type, media_id, node_index, uri, tags }`. The lock says what is a file; nothing else marks a node playable.

---

## 9. Actions & Interfaces (`d.action`, `d.interface`)

```lua
d.action("open_anchor_manifest", {
  label = "Open Anchor Manifest",
  description = "Edit album.toml",
  run = function(ctx)
    for _, item in ipairs(ctx.items) do
      -- item = { type = "album", id = "...", path = "/abs/dir", lock = { album = { ... } } }
      d.system({ "xdg-open", item.path .. "/album.toml" }, { detach = true })
    end
  end,
})

d.interface("default", {
  label = "Web UI",
  run = function(ctx)
    -- ctx = { config = ..., roots = { album = "~/library", ... } }
  end,
})
```

- **No built-in actions.** Datamile ships none; every action (including `open_terminal`, `open_folder`, `open_lock`, `open_config_in_terminal`) is declared by the user in `init.lua`.
- `label` and `description` are forwarded to clients in the `INIT` / `LOGIC_UPDATE` payload, so an interface can render an action it knows by name and hide it otherwise.

---

## 10. WebSocket & HTTP API Contract

### 10.1 WebSocket (`/ws`)
**Server → Client:**
- `INIT`: `{ type = "INIT", media = { "<type>": { "<id>": SurfaceItem } }, manifest, actions, shelves, ui_state }`
- `MEDIA_UPDATED`: `{ type = "MEDIA_UPDATED", media_type, updated: { "<id>": SurfaceItem }, removed: ["<id>"], shelves }`
- `LOGIC_UPDATE`: `{ type = "LOGIC_UPDATE", manifest, actions }`
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
- `GET /api/media/{type}/{*id}?depth=<n>` → Literal `<type>.lock.json` from disk (WYSIWYG), optionally truncated to `depth` levels of `nodes`.
- `POST /api/media/{type}/update/{*id}` → Forces recompile of one `(type, id)` root.
- `POST /api/media/{type}/play/{*id}?start=&count=&offset=` → Plays playable slice in MPD.
- `POST /api/media/{type}/queue/{*id}?start=&count=` → Appends playable slice to MPD queue.
- `POST /api/playback/{jump/{index},next,prev,stop,clear,toggle-pause}`
- `POST /api/actions/{name}` (accepts `type`, `id`, `query`, `playing`, `directory`, `recursive`, `library`, plus option params; resolves targets into `ctx.items`)
- `GET /api/images/{file}?size=&filter=&quality=` & `POST /api/images/cache`
- `POST /api/state`, `POST /api/update` (NDJSON event stream), `GET /api/identity`, `POST /api/select`, `POST /api/reset`

---

## 11. CLI Contract (`datamile`)

Global flags: `--config <dir>`, `-v` (log filter to debug, which is what shows `Unchanged`), `--silent` (print nothing).

- `datamile server` — runs `datamile-server` backed by `datamile-media`, on `server.port` from the config. One daemon per user, enforced by a kernel lock at `~/.local/state/datamile/instance.lock` (`$HOME` only; `XDG_STATE_HOME` is ignored on purpose).
- `datamile compile <path> [--type <media_type>] [--pretty]` — renders the lock and prints it. It never writes a lock file and takes no instance lock; rendered images still land in the content-keyed cache.
- `datamile update [<path>] [--type <media_type>] [--force] [--jobs <n>]` — full discovery plus incremental compilation. A free instance lock means no daemon: the pass runs here holding the lock. A held lock means the configured port is asked; the daemon's `config_path`, version and reload state must match, or the command fails instead of writing locally. Any failure exits non-zero, including under `--silent`.
- `datamile x <action> [--type <media_type>] [--playing | --id <id> | --query <q> | --directory <d> | --recursive <d> | --library] [-- <args>]` — selection comes from the daemon's index when one is live on the same config, otherwise from a local scan through the same backend. The action always runs in the CLI.
- `datamile interface [<name>]`
```

