# Multi-lock entity dirs

Status: plan. Not approved, not implemented. Records the 2026-11-24 decision
that one directory may hold several media types' locks, and what that costs
in discovery, indexing, lookups, lock lifecycle, and `datamile x` target
resolution.

`specs/actions/spec.md` defines `d.action`, `ActionSet`, `ctx`, and `/api/x`;
this document owns entity identity, the single write path, lock freshness, and
target resolution for `datamile x`.

## Core design rules

Four rules replace the old `1 dir == 1 lock` assumption throughout:

1. **Identity is `(media_type, dir)`, never `dir` alone.** Every index,
   watcher target, dependency edge, removal, and update scope carries
   `EntityKey { media_type, dir }`. No function removes, updates, or indexes an
   entity by bare path.
2. **One write path.** All compilation and state writes flow through one
   serialized pipeline (`run_server_update` when the daemon is live,
   `InstanceLock` when a CLI runs offline). The watcher resolves changed
   entities and submits requests to that pipeline instead of running a second
   compiler.
3. **Sources are not outputs.** Input files and `*.lock.json` outputs stay
   strictly separated: fingerprinting skips every lock file, lock freshness
   is tracked via `lock_stat: FileStat { mtime, size }`, and compiling may never
   read any lock file.
4. **Facts in the index, meaning in the consumer.** Shared audio files and image
   addresses keep all owners in the index; each consumer names the media type it
   owns.

## Intent

The same files can be several media types at once. One folder of FLACs can be an
`album` (generic playback shape), a `vinyl` (`sides` node holding `tracks`),
and a `cd` (CD-only metadata). These are different compilations of the same
audio, not different audio.

```text
/media/abbey-road/
  album.toml  cd.toml  vinyl.toml
  01 - Come Together.flac   02 - Something.flac   ...
  album.lock.json  cd.lock.json  vinyl.lock.json
```

Keeping one media type for all three shapes forces unrelated node trees into
one lock; splitting into three directories forces audio and artwork to live
outside their entity dirs. Multi-lock dirs let each media type compile its own
shape beside the same source files.

## The rule

- A dir may hold `<media_type>.lock.json` for any number of media types. Each
  lock is the entity of its own media type; ids are per media type and need not
  agree.
- Within one `d.media.root`, `select` must not return two dirs where one is a
  subfolder of the other. Same-root nesting stays an error.
- Between different `d.media.root`s, shared dirs and nesting are allowed.

```text
/media/films/Inception (2010)/          legal: two roots, nested
  movie.toml  inception.mkv  movie.lock.json
  soundtrack/
    album.toml  *.flac  album.lock.json

/media/albums/Abbey Road/               illegal: one root, nested
  album.toml  ...  album.lock.json
  disc2/
    album.toml  ...  album.lock.json
```

## What it changes

### 1. Discovery

`core/datamile-media/src/schema.rs` refuses both shapes today:

- `claimed: HashMap<PathBuf, String>` spans the whole pass, so two roots that
  select the same dir collide even when their media types differ
  (`root directory X is claimed by both 'album' and 'movie'`).
- `reject_nesting` sorts and compares adjacent dir pairs across all media types.
  Its docstring justifies this with "so every file belongs to at most one of
  them"; that guarantee is what this plan replaces.

Changes to implement:

- Remove the cross-type `claimed` collision check (per-root uniqueness is already
  enforced inside `select_media_roots` via `seen`).
- Scope `reject_nesting` to compare dirs only within each `media_type`.
- Update `reject_nesting`'s docstring and invert the cross-type collision test in
  `core/datamile-media/src/tests.rs`.

### 2. Entity identity & index

Define `EntityKey` in `datamile-cache` (re-exported by `datamile-media` and
`datamile-server`) so `datamile-lua`, `datamile-media`, `datamile-server`, and
`datamile` share one identity type without circular dependencies:

```rust
/// Canonical identity of one compiled media entity: `(media_type, dir)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct EntityKey {
    pub media_type: String,
    pub dir: PathBuf,
}
```

Changes to implement:

- **`LogicEngine` removal (`core/datamile-logic/src/engine.rs`)**: replace
  `remove_entity_by_path(&mut self, path)` with
  `remove_entity_by_type_and_path(&mut self, media_type: &str, path: &Path)` and
  update callers in `update.rs`, `watcher.rs`, `validate_and_cleanup_paths`, and
  `core/datamile-logic/src/tests.rs`.
- **Force update (`core/datamile-media/src/backend.rs`)**:
  `force_update_entity` sets `media_type: Some(media_type.to_string())` on
  `UpdateRequest` (today it calls `UpdateRequest::force(...)` with `None`, which
  would recompile every media type in a shared dir).
- **Update scope (`core/datamile-media/src/deps.rs`, `update.rs`, `core/datamile-server/src/backend.rs`)**:
  replace `UpdateScope` + `PendingUpdate.media_type` with one clean enum:
  ```rust
  pub enum UpdateScope {
      All { media_type: Option<String> },
      Paths { paths: HashSet<PathBuf>, media_type: Option<String> },
      Entities(BTreeSet<EntityKey>),
  }
  ```
  Merge rules: `All { None }` absorbs everything; `All { Some(a) }` + `All { Some(b) }`
  becomes `All { None }` when `a != b`; `Entities` + `Entities` unions both sets;
  `Entities` + `All`/`Paths` widens cleanly (or filters `Entities` into the union).
  `UpdateRequest` (`datamile-server/src/backend.rs`) gains
  `#[serde(default)] pub entities: Vec<EntityKey>` so `POST /api/update` remains
  backwards-compatible.

### 3. Dependency index

Today `core/datamile-media/src/deps.rs` uses
`ExternalIndex = BTreeMap<PathBuf /*external file*/, BTreeSet<PathBuf /*dir*/>>`
and `index_root` calls `unindex_root(dir)` first, erasing sibling types in the
same dir.

Changes to implement:

- Key owners as `EntityKey`:
  `pub type ExternalIndex = BTreeMap<PathBuf, BTreeSet<EntityKey>>;`
- `BTreeSet<EntityKey>` guarantees idempotency by type.
- `index_entity(index, key, external)` removes `key` from existing buckets
  (`unindex_entity(index, key)` via `retain`) and inserts `key` for each file in
  `external`. No separate `reverse` map is needed.
- `rebuild_external_index` already walks `state.media_roots()` ->
  `(media_type, init)` and builds `EntityKey { media_type, dir }` entries.

### 4. Watcher & single write path

Today `core/datamile-media/src/watcher.rs` compiles, prepares entities, updates
`LogicEngine`, updates `ExternalIndex`, and broadcasts `MEDIA_UPDATED` on its own
without acquiring `shared.update_lock`—racing with startup auto-update and CLI
passes—and has two multi-lock bugs (`removed_roots` checks bare `dir`, and
`collect_external_dependencies` filters hits through the boot-time `state.roots`
snapshot, missing runtime-added entities).

Under rule 2 (**One write path**):

- Config changes reload config and manifest as today, then enqueue
  `UpdateScope::All { media_type: None }` if needed.
- Non-config filesystem events in `watcher.rs` do not compile or write locks/state:
  - External dependency hits read `ExternalIndex` directly into a
    `BTreeSet<EntityKey>` (no `state.roots` filter).
  - Watch-root path hits scan affected `(media_type, init)` pairs via
    `known_root_ancestor` and `select_roots` to collect changed/new `EntityKey`s,
    or enqueue scoped `Paths` when an ancestor directory is deleted so removals
    are reconciled cleanly.
- The watcher submits the collected targets to `run_server_update`. All locking,
  compilation, ingestion, `ExternalIndex` updates, `state.json` writes, and
  `MEDIA_UPDATED` broadcasts happen in one place.
- `active_writes` remains keyed on `<dir>/<media_type>.lock.json` strictly as a
  latency optimization to skip re-evaluating self-written lock events.

### 5. Playable lookups

`playable_lookup` (`core/datamile-logic/src/engine.rs`) currently maps URI ->
one `PlayableLookupItem`, clobbering on upsert and erasing on delete.

Changes to implement:

- Store `HashMap<String, Vec<PlayableLookupItem>>` (keeping `media_type`,
  `media_id`, `node_index`, `node_path`, `path`, `meta`).
- Upserting replaces any existing entry in the URI bucket with the same
  `(media_type, media_id)` and keeps all other owners.
- Removing an entity drops only items matching `(media_type, media_id)` and
  removes the URI key only when its owner list becomes empty.

### 6. Image lookups

`image_lookup` (`core/datamile-logic/src/engine.rs`) currently maps `address` ->
one `ImageLookupItem` (carrying `entity_dir`, `path`, `fit`, `filter`,
`target_width`, `target_height`).

Changes to implement:

- Store `HashMap<String, BTreeMap<(String, String), ImageLookupItem>>` keyed by
  `(media_type, media_id)` per `address` so removing one entity cannot orphan
  another owner's image address.
- `GET /api/images/{file}` (`core/datamile-server/src/api/images.rs`) selects
  the owner with the lowest `(media_type, entity_dir)` deterministically when
  rendering a missing master QOI.

## Freshness and lock lifecycle

The lock is a compiled output that outlives its entity (`lua-api.md` §7).
Everything below is **new behaviour to implement** (today `walk_files` walks
locks too, `RootState` carries only `files`, and `CompiledRoot` carries no lock
metadata).

```text
state entry for album/abbey-road
  files:     { album.toml: {mtime,size}, 01 - Come Together.flac: {mtime,size},
               /media/abbey-road/cover.jpg: {mtime,size} }
  lock_stat: { mtime, size }   -- of album.lock.json, recorded without hashing
```

### No lock is an input

`walk_files` (`core/datamile-cache/src/stat.rs`) skips every file whose name
ends with `.lock.json`. Otherwise the compiled output becomes an input,
dirtying its own entity and sibling types in the same directory.

### Lock freshness via `FileStat`

`RootState` (`core/datamile-cache/src/stat.rs`) adds:

```rust
#[serde(default)]
pub lock_stat: Option<FileStat>,
```

No `blake3` hashing is performed on locks anywhere. During planning
(`plan_work`), an entity is dirty if any of the following hold:

- `entry` is missing, or `entry.lock_stat` is `None`.
- `walked` / external files fail `root_is_dirty`.
- `<dir>/<media_type>.lock.json` is missing on disk.
- The lock file's `FileStat { mtime, size }` differs from `entry.lock_stat`
  (catches hand-edits, truncations, and deletions at `stat()` cost).
- `serde_json::from_slice::<Value>` fails on the lock file (catches partial/corrupt
  writes).

A lock has no version key. Its only top-level key is the media type, so
`lock[media]` is the entity, and schema changes are handled by recompiling on
config digest change.

### `lock_stat` is recorded for every compiled root

`CompiledRoot` (`core/datamile-media/src/compiler.rs`) gains
`pub lock_stat: FileStat`, populated via `FileStat::from(&fs::metadata(&lock_path)?)`
inside the compile step immediately after writing (when `modified == true`) **or**
verifying the existing lock (when `modified == false`).

Both `update.rs` and `watcher.rs` record `lock_stat` into `RootState` for
**every** compiled root—never skipping `!root.modified` roots before writing
`state.json`. Only the `MEDIA_UPDATED` WebSocket broadcast skips unchanged known
entities.

Three conditions guarantee loop termination:

1. `lock_stat` is recorded from the lock on disk for every compiled root in the
   single write pipeline.
2. No lock file is an input (`walk_files` skips `*.lock.json`).
3. One write path owns compilation and state persistence.

### Locks are never deleted, never renamed

A lock left behind by a dropped entity stays under its normal name. Its state
entry in `state.json` (or `LogicEngine`) decides liveness, so the leftover is
inert:

```console
$ ls /media/gone
album.lock.json   # album.toml deleted, state entry removed
$ datamile x open_terminal --dir /media/gone --media album
error: entity ('album', '/media/gone') is not in the entity index; run `datamile update`
$ datamile x open_terminal --dir /media/gone
error: no indexed entity in '/media/gone'; run `datamile update`
```

Pruning leftover locks is deliberately rejected: a temporary discovery
regression (e.g. an unmounted path or a `select` bug) would wipe compiled locks
across the library.

### All lock reads during compile are an error

```text
/media/boxset/package.toml
  tags = { tracks = d.fs.read_json("../disc1/album.lock.json").tags.tracks }

error: compile cannot read lock file 'disc1/album.lock.json'; a lock is never an input.
       Read the source manifest or audio files instead.
```

Enforced in `EngineContext` (`core/datamile-lua/src/lib.rs`) using an RAII
`CompileGuard` created at the start of `compile_root_impl` and dropped on return
(including `?` error paths, so a leaked flag never poisons a reused VM). While
the guard is active, any `*.lock.json` path (including the entity's own lock)
passed to `d.fs` (`read`, `read_json`, `read_toml`, `read_lines`, `read_dotenv`,
`exists`, `first`, `files`, `dirs`, `roots`, `scandir`) fails as a config error.
(`d.json` only has `encode`/`decode` and touches no paths.)

Note: `d.fs` hooks run in Lua; raw Lua `io.open` is outside `d.fs` and
unsupported for dependency tracking.

### Read-only paths

If a dirty or modified entity cannot write its lock because the path is
read-only (`PermissionDenied` or `ReadOnlyFilesystem` on `write_atomic`),
`compile_root_impl` fails **that entity only** with:

```text
/media/readonly/album.lock.json could not be updated; datamile cannot recompile on a read-only path
```

Other entities, media types, and the running daemon continue unaffected.

### Migration

Existing `state.json` files lack `lock_stat` (`Option<FileStat>` deserializes as
`None`), and excluding `*.lock.json` changes `walk_files`. Every entity is
recompiled cleanly once on the first `datamile update` pass after upgrading, then
settles.

## Consumers

The Rust binary is media-agnostic apart from MPD, which owns the
`album (root) -> track (node)` shape only and will be retired when `datamile-mpv`
replaces MPD. Until then:

- `mpd_bridge` (`core/datamile-server/src/mpd_bridge.rs`) and `playing_target`
  (`core/datamile-media/src/backend.rs`) filter `playable_lookup` for `"album"`
  as an explicit, temporary documented exception.
- A future `datamile-mpv` crate will own playback sessions, resolve its own
  entities, and retire this exception.

## Target resolution

`datamile x` (and `/api/x` / `/api/x/resolve`, sharing one Rust resolver)
resolves entity targets using `(media_type, dir)`:

- **`--dir <entity-dir>...`**: repeatable; applies `~` expansion and
  `std::fs::canonicalize` before matching against `EntityKey.dir` / `state.json`.
  A path that cannot be canonicalized (missing or deleted) fails for that dir.
  There is no subdirectory search and no walking up.
- **`--media <media_type>...`**: optional, repeatable. Any value not in the
  registered `MediaManifest` fails before any dir is resolved.
- **When `--media` IS provided**: `--media` is a **literal selector** of
  `<canon_dir>/<media>.lock.json`. There is no execution guard—even if the
  action did not declare that media type in `d.action`, the pair resolves and
  runs if indexed.
- **When `--media` is OMITTED**:
  - **Generic action (`action.media == nil`) + no `--dir`**: runs once targetless
    (`ctx.lock = nil`, `ctx.media = nil`).
  - **Generic action + `--dir` given**: runs if `canon_dir` has exactly 1
    indexed entity; errors if 0 or 2+ (`"ambiguous: pass --media"`).
  - **Scoped action (`action.media` declared)**: omitted `--dir` defaults to `.`
    (on CLI; HTTP `/api/x` requires `dir` for scoped actions and returns `400` if
    omitted). Intersects the indexed entities at `canon_dir` with
    `action.media` (**Option B**):
    - 1 match -> runs that `(canon_dir, media)` pair.
    - 2+ matches -> error listing the matching media types (`"ambiguous: pass --media"`).
    - 0 matches -> error: if `canon_dir` has no indexed entities at all, name
      `datamile update`; if it has other media types not in `action.media`, name
      the indexed types and note `"pass --media to select explicitly"`.
- **Cross product & exit code**: runs are `(dir, media)` in argument order; one
  result per run. Failures are collected so remaining pairs still run, and the
  CLI exits non-zero if any run failed.
- **Entity index as truth (online & offline)**:
  - `x` never compiles and never checks staleness (`datamile update` ensures
    strict freshness; `x` uses the last compiled lock in the index).
  - **Daemon live**: resolves via `POST /api/x/resolve` against the daemon's
    in-memory index (`LogicEngine`).
  - **Offline (`InstanceLock::holder()` is `None`)**: evaluates `init.lua` only
    (**no `discover_roots`, no `select` walk, no `scan_library`, no `evaluate_logic`**),
    reads `state.json` per `(media_type, init)` to verify indexed `EntityKey`s,
    and reads only the targeted `<canon_dir>/<media_type>.lock.json` file(s).
    Targetless generic actions do not read `state.json` or require library
    mounts to be present.

## Edge cases

| Case | Behaviour |
| --- | --- |
| generic action, no `--dir` and no `--media` | runs once with `ctx.lock = nil, ctx.media = nil` |
| generic action, `--dir` with 1 entity, no `--media` | runs it with `ctx.lock` and `ctx.media` |
| generic action, `--dir` with 2+ entities, no `--media` | error: ambiguous, pass `--media` |
| scoped action (`album`), dir has `album` + `vinyl`, no `--media` | runs `album` (Option B intersection) |
| scoped action (`album, vinyl`), dir has `album` + `vinyl`, no `--media` | error: ambiguous, pass `--media` |
| scoped action (`album`), dir has `movie` only, no `--media` | error: no matching entity; pass `--media` to select explicitly |
| any action, explicit `--dir X --media Y` (indexed) | runs `Y` regardless of action's declared `media` |
| unknown dir with `--media` | error naming `datamile update` |
| unknown dir, no `--media` | error naming `datamile update` |
| dir deleted or non-existent | error for that dir |
| unregistered `--media` value | error before any dir is resolved |
| nested dirs inside one media root | discovery error, unchanged |
| nested dirs across media roots | allowed |
| lock missing, unparseable, or `lock_stat` mismatch | recompiled; edits discarded |
| compile reads any `*.lock.json` via `d.fs` | config error via `CompileGuard` |
| state file from before `lock_stat` existed | one full recompile, then stable |
| read-only library with dirty entity / modified lock | entity fails loudly; other entities proceed |
| lock write during a pass | `active_writes` skips or pipeline sees matching `lock_stat` |
| same file playable under two types | both owners kept; MPD asks for `"album"` |
| same image address under two entities | shared ownership; deterministic lowest `EntityKey` owner |

## Alternatives Considered

- **Content `blake3` digest on locks (`lock_hash`) instead of `lock_stat`**:
  Hashing every lock JSON on every `update` pass detects same-size, same-mtime
  edits, at the cost of reading and hashing every lock file on disk during
  planning and adding `blake3` to `datamile-media`. `FileStat { mtime, size }`
  matches `datamile-cache`'s existing change detector at `stat()` cost while
  `serde_json::from_slice` catches truncated partial writes.
- **Separate `reverse` map in `ExternalIndex`**: Storing `HashMap<EntityKey,
  Vec<PathBuf>>` speeds up `unindex_entity` at the cost of maintaining two
  synchronized maps and duplicating `RootState.files`. External dependency maps
  are small (cover art and stray metadata), so `retain` over `BTreeMap<PathBuf,
  BTreeSet<EntityKey>>` keeps one source of truth.
- **Keep watcher compilation independent behind a per-entity mutex**:
  Serializing `watcher` and `update` per entity instead of unifying on
  `run_server_update` preserves duplicate ingest/broadcast code and risks
  deadlocks against `state_writer`. Routing the watcher into `run_server_update`
  eliminates the duplicate pipeline.

## Risks & Limits

- **Cross-root nesting `walk_files` amplification**: Because `walk_files` is
  recursive, an outer entity (e.g. `movie` at `/media/films/Inception`) that
  contains an inner entity (`album` at `.../soundtrack`) includes the inner
  dir's non-lock files in its `RootState.files`. Editing a soundtrack track will
  also mark the parent `movie` dirty once (while `collect_media_changes` uses the
  nearest known ancestor per media type). Acceptable for occasional nested
  collections; avoidable in the future by teaching `walk_files` a skip list of
  other root dirs if deep trees become common.
- **Watcher coalescing & burst control**: Rapid multi-file edits or deletes must
  coalesce inside `UpdateScope::Entities(BTreeSet<EntityKey>)` and `PendingUpdate`
  rather than spawning full passes per event. Only missing/unrecognized dirs
  trigger scoped `(media_type, init)` discovery.
- **Stale-entity debounce window on `x`**: Between a file edit and the 100ms
  watcher debounce + recompile completing, `x` (online or offline) reads the
  prior lock from the index. Callers needing guaranteed freshness run `datamile update`
  first.
- **Lock-failure isolation**: Compile or read-only failures record `UpdateEvent::Failed`
  with `Some(key)` and leave the entity's previous `RootState` entry untouched so
  the next pass retries; they never abort sibling entities or crash the daemon.

## Docs and invariants

- Update `docs/vocabulary.md`: document `EntityKey`, `lock_stat`, shared entity
  dirs, `x` reading index (`state.json` offline) without staleness checks, and
  remove `POST /api/select`.
- Update `docs/lua-api.md` §4.1, §6, §7, and §11: reflect per-type uniqueness
  and nesting, multi-lock dirs, compile-time ban on reading `*.lock.json`, and
  `--dir` / `--media` target resolution.
- Update `core/datamile-media/src/schema.rs` docstrings (`reject_nesting`).

## Implementation

### Files

- `core/datamile-cache/src/stat.rs` and `lib.rs`: define `EntityKey`, skip
  `*.lock.json` in `walk_files`, add `lock_stat: Option<FileStat>` to
  `RootState`, update `root_is_dirty` / tests.
- `core/datamile-logic/src/engine.rs` and `tests.rs`:
  `remove_entity_by_type_and_path`, multi-owner `playable_lookup` (`Vec<PlayableLookupItem>`),
  multi-owner `image_lookup` (`BTreeMap<(String, String), ImageLookupItem>`).
- `core/datamile-lua/src/lib.rs` and `functions/fs/mod.rs`: `CompileGuard` in
  `EngineContext` rejecting `*.lock.json` reads/walks during compile.
- `core/datamile-media/src/schema.rs`: per-type nesting only, remove cross-type
  `claimed` collision check.
- `core/datamile-media/src/compiler.rs`: wrap `compile_root_impl` in
  `CompileGuard`, compute `CompiledRoot.lock_stat`, format read-only write errors.
- `core/datamile-media/src/deps.rs`: `ExternalIndex = BTreeMap<PathBuf, BTreeSet<EntityKey>>`,
  `UpdateScope` enum, `record_roots` taking `(dir, files, lock_stat)`.
- `core/datamile-media/src/update.rs`: `plan_work` checks `lock_stat` + JSON
  parse, records `lock_stat` for all compiled roots (modified or not), handles
  `UpdateScope::Entities`.
- `core/datamile-media/src/watcher.rs`: remove duplicate compile/ingest/broadcast
  pipeline; resolve `EntityKey`s (using `ExternalIndex` directly without
  `state.roots`) and submit to `run_server_update`.
- `core/datamile-media/src/backend.rs`: update `force_update_entity`
  (`media_type: Some(...)`) and `playing_target` (query `"album"`).
- `core/datamile-media/src/tests.rs`: invert cross-type collision test, add
  multi-lock, `lock_stat`, read-only, `CompileGuard`, and watcher-pipeline tests.
- `core/datamile-server/src/backend.rs` and `lib.rs`: `UpdateRequest.entities`,
  re-export `EntityKey`.
- `core/datamile-server/src/mpd_bridge.rs`: filter `playable_lookup` for `"album"`.
- `core/datamile-server/src/api/images.rs`: deterministic lowest-`(media_type, dir)`
  owner selection.
- `core/datamile/src/main.rs` and `tests`: shared target resolution against
  daemon `/api/x/resolve` or offline `state.json` (no `discover_roots` / `scan_library`).

### Phases

Each phase compiles and passes `devenv shell -- lint` and `devenv shell -- tests`.

1. **Identity & lookups**: `EntityKey` in `datamile-cache`, `remove_entity_by_type_and_path`
   and multi-owner `playable_lookup` / `image_lookup` in `datamile-logic`,
   `mpd_bridge` `"album"` filter, deterministic image owner, `force_update_entity`
   fix, per-type discovery in `schema.rs`.
2. **Freshness & compile guard**: `walk_files` skipping `*.lock.json`,
   `RootState.lock_stat`, `CompiledRoot.lock_stat`, `CompileGuard` in
   `datamile-lua`, read-only failure isolation, record-on-all-compiled in
   `update.rs`.
3. **Single write path**: `ExternalIndex` on `EntityKey`, `UpdateScope` +
   `UpdateRequest.entities`, route `watcher.rs` through `run_server_update`.
4. **Target resolution**: shared resolver for `datamile x` (offline `state.json`
   path without `select`/`scan_library`) and daemon index, plus docs.

### Tests

- `datamile-cache`: `walk_files` skips `*.lock.json`; `RootState` without
  `lock_stat` deserializes as `None`; `lock_stat` mismatch marks dirty.
- `datamile-logic`: `remove_entity_by_type_and_path` leaves sibling type in
  shared dir intact; multi-owner `playable_lookup` & `image_lookup` survive
  removing one owner.
- `datamile-lua`: `CompileGuard` blocks `d.fs.read_json` / `read_toml` / `read`
  on foreign and own `*.lock.json`, and clears on compile error (`?` return).
- `datamile-media`: two media roots claiming one dir and nesting across roots
  both succeed; same-type nesting fails; `ExternalIndex` retains both types on
  shared external edit; dirty root compiling to identical bytes updates
  `lock_stat`; read-only lock write fails only that entity; watcher edit submits
  into `run_server_update` without racing.
- `datamile-server`: `images.rs` picks deterministic owner; `mpd_bridge` picks
  `"album"` from multi-owner URI.
- `datamile`: CLI integration tests for every row of *Edge cases* (including
  offline `state.json` parity when a manifest was deleted leaving an orphan lock).

### Breakage inventory

- `core/datamile-logic/src/tests.rs` (`remove_entity_by_path` assertion) updates
  to `remove_entity_by_type_and_path`.
- `core/datamile-media/src/tests.rs` (`discovery_rejects_two_types_claiming_one_directory`,
  `discovery_rejects_nested_roots`) updates to assert cross-type sharing/nesting
  succeeds while same-type nesting still fails.

## Non-goals

- Playback sessions and the `datamile-mpv` crate.
- Lock-as-input: an entity whose tags come from another entity's lock needs a
  build graph with wave recompilation and cycle rejection.
- Pruning dead locks (see *Locks are never deleted*).

