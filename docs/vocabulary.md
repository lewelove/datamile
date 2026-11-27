# Datamile Vocabulary

The canonical words. When a term is overloaded, the preferred reading is
named first and the others are listed under "Conflicts and retired terms".

## Config and scripting

- **config dir** — the directory passed with `--config`, containing
  `init.lua`. Root for `require` and relative storage paths.
- **`init.lua`** — the entry script. Runs at startup and on config reload.
- **`require`** — loads a Lua file relative to the config dir.
- **`REGISTRY`** — the Lua table where `d.*` declarations are stored while
  `init.lua` runs.
- **`d` / `datamile`** — aliases of the same global API table.
- **`d.config`** — server, storage and MPD settings.
- **interface** — a named Lua function that launches a client.
- **action** — a named Lua function invoked by name, optionally scoped to
  media types.

## Media schema

- **media root** (`d.media.root(name, def)`) — the declaration of a media
  type and how its entities are discovered. Its name is the media type.
- **media type** — the name of a media root (`album`, `book`, `movie`).
  The key used in locks, indexes, actions and the WS payload.
- **media entity** — one instance of a media root: one entity key, one
  lock, one id. Entities of different media types may share an entity dir.
  The short form **entity** means the same.
- **entity key** — `(media type, entity dir)`: the unique identity of one
  media entity.
- **entity dir** — the canonical directory holding an entity's
  `<media_type>.lock.json`. One dir may hold several media types' locks,
  one entity per media type.
- **init** — the absolute root directory a media root scans for entities.
  It is not `init.lua`.
- **select** — returns the directories that are entities.
- **id** — returns an entity's stable id within its media type.
- **node** (`d.media.node`) — a nested part of an entity (`track`,
  `bonus`, `season`, `episode`).
- **tag** — a scalar or array field on an entity or node, produced by a
  Lua function at compile time.
- **object** — a structured field on an entity or node (`info`), produced
  lazily by a Lua function.
- **image** — a rendered image namespace (`main`), produced from sources.
- **location** — where an entity or node lives: `{ kind, path }`, plus
  `mtime` and `byte_size` for files. A root entity's path is its canonical
  absolute dir path, always present in the lock.
- **playable URI** — the MPD-facing URI of a playable file: the entity's
  `location.path` joined with the node's own relative `location.path`, with
  `mpd.music_directory` subtracted. Any node whose `location.kind` is `file` is
  playable; no flag marks it.

## Compilation and artifacts

- **manifest** — the source file the compiler reads (`album.toml` in the
  album config). Called **entity manifest** when it could be confused with
  the logic manifest.
- **lock** — the compiled `<media_type>.lock.json`. Always an output, never
  an input.
- **compile** — evaluate an entity manifest into a lock.
- **discovery** — find entity dirs from a media root's `select`.
- **update** — discovery plus incremental compilation.
- **pass** — one update run.
- **cache root** — content-keyed image and object cache.
- **state file** — per-media-root bookkeeping under the state cache dir,
  keyed by canonical entity dir.
- **entity index** — the daemon's catalog of compiled entities built from
  state entries and locks: one record per entity with its playable URIs and
  CAS image references. Action targets and playback resolve through it. A lock
  sitting on disk without a state entry is not in the index.
- **stale** — an entity whose state entry no longer matches disk: an input
  file changed, an input or the dir vanished, the lock file changed or is
  missing, or no entry exists. Update passes recompile stale entities and skip
  fresh ones.
- **hot reload** — a file event triggers recompilation or a config reload.

## Logic

- **logic** — the per-entity evaluated match and sort data, plus the
  declared logic units.
- **logic manifest** — `LogicManifest`: the declared filters, groupers,
  orders, libraries, shelves and cabinets.
- **filter** — a named predicate over an entity.
- **grouper** — a named value or list of values per entity; builds
  sidebar buckets.
- **order** — a named sort key per entity.
- **library** — a named logic view: a set of filters, groupers and orders.
  Not the collection.
- **shelf** — a named saved set of entities with its own match and sort.
- **cabinet** — a named group of shelves and orders shown as a UI section.
- **view** — the intersection of library, filter and search, then ordered.
- **shelf view** — a shelf's ordered entity ids.

## Actions and interfaces

- **action** — a named Lua function with `run`, and optional `media`, `label`
  and `description`.
- **generic action** — `media = nil`; not tied to a media type. Runs
  without a target or against a resolved entity.
- **scoped action** — `media` is a non-empty string or a list of them; used
  for validation, client presentation, and default selection when `--media`
  is omitted.
- **action map** — the resolved catalog shipped to clients: a map keyed by
  canonical action name with `{ label, description, media? }`.
- **ctx** — the single table passed to an action's `run(ctx)`: `{ lock,
  media, options, isatty }`. `ctx.lock` is the targeted entity's parsed lock
  (`{ <media type>: { id, location, tags, objects, images, nodes } }`), and
  `ctx.media` is its media type; both are `nil` when targetless.
- **run** — one execution of an action for a single `(dir, media)` pair (or
  one targetless execution); also the Lua function field on `d.action`.
- **option** — the trailing `-- <args>` string array passed as `ctx.options`.

## Server and transport

- **server** / **daemon** — the Axum process.
- **instance lock** — one daemon per user, a kernel lock at
  `~/.local/state/datamile/instance.lock`.
- **identity** — the daemon's config path, version and reload state; the
  CLI checks it before delegating.
- **`INIT`**, **`LOGIC_UPDATE`**, **`MEDIA_UPDATED`**, **`VIEW_DATA`**,
  **`SHELF_DATA`**, **`GROUP_RESULT`**, **`MPD_STATUS`** — WS messages.
- **MPD** — the music player daemon; **queue** and **playback** are its
  state.

## Client

- **interface** — a client, for example the web app.
- **drawer** — the entity detail panel.
- **queue view** — the now-playing view.
- **cabinet**, **library**, **filter**, **grouper**, **order**, **shelf**
  — UI mirrors of the logic units.

## Conflicts and retired terms

- **library** — overloaded: a logic unit (`d.library`) versus the whole
  collection. Prefer **logic library** for the unit and **collection** for
  the whole.
- **root** — overloaded: a media root's `init`, an entity dir, or the
  filesystem root. Prefer **media root**, **entity dir**, or **entity key**;
  avoid bare "root".
- **manifest** — the entity manifest (`album.toml`) versus the logic
  manifest. Always qualify.
- **index** — overloaded: the **entity index** (entities the daemon knows)
  versus the **dependency index** (external file → entities that read it).
  Qualify it.
- **state entry** — one entity's row in a state file: the input files under
  its dir except `*.lock.json`, plus its external dependencies, each with
  mtime and size, plus the mtime and size of its lock file. This is what
  makes staleness checkable without compiling.
- **media manifest** — the evaluated set of media roots, held in Rust as
  `MediaManifest`.
- **getter** — a `__DATAMILE_GET_*` Lua function Rust calls after `init.lua`
  returns, to pull declarations out of `REGISTRY`.
- **fingerprints** — the map from file to mtime and size inside a state
  entry.
- **item** — retired (`ctx.items`). An action now receives `ctx.lock` and
  `ctx.media` for one entity at a time.
- **select** (action route) — retired (`POST /api/select`, `POST /api/actions/{name}`).
  Actions resolve and run via `POST /api/x`.
- **type** — in `datamile x`, replaced by **media** (`--type` → `--media`).
  `compile` and `update` keep `--type`; code keeps `media_type` for the key.
- **directory** — in `datamile x`, replaced by **dir** (`--directory` →
  `--dir`), which is an entity dir.
- **`--library`** — retired. It meant "every entity under the first init
  dir". Do not reuse the name; `--dir` is the target.
- **`--all`** — retired. No batch flag; batch is `--dir` with a shell
  glob or an explicit list.
- **`--playing`**, **`--id`**, **`--query`**, **`--recursive`** —
  retired. The action target is an entity dir.
- **root media**, **branch media**, **leaf media** — vocabulary from
  `notes/root-branch-leaf-media.md`. Current code says **media root** and
  **node**.
- **album** — one media type, not a synonym for entity.
