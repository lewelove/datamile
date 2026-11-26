# Multi-lock entity dirs

Status: specification.

`specs/actions/spec.md` defines `d.action`, `ctx`, and `POST /api/x`; this
document owns entity identity, the single write path, lock freshness, CAS
image lookup ownership, `CompileGuard`, and target resolution.

## Intent

The same source files can belong to multiple media types at once. One folder of
FLACs can compile as an `album` (tracklist view), a `vinyl` (`sides` -> `tracks`
view), and a `cd` (CD metadata). Keeping one media type for all shapes forces
unrelated node trees into one lock; splitting directories forces audio and
artwork to live outside their entity dirs. Multi-lock entity dirs let each media
type compile its own `<media_type>.lock.json` beside the same source files.

## Goals

- Allow one directory to hold `<media_type>.lock.json` for multiple media types
  simultaneously while keeping same-type nesting forbidden.
- Route all lock/state writes through one serialized pipeline (`run_server_update`
  on the daemon, `InstanceLock` for offline `datamile update`).
- Enforce strict separation between source inputs and `*.lock.json` outputs via
  one `d.fs` choke point and `walk_files` exclusion.
- Preserve shared audio and CAS image ownership across sibling media types.
- Resolve `POST /api/x` targets deterministically via `(media_type, dir)`.

## Non-goals

- Playback sessions and the `datamile-mpv` crate.
- Lock-as-input (reading another entity's lock during compile).
- Pruning leftover `<media_type>.lock.json` files when manifests are deleted.

## Core rules

1. **Identity is `EntityKey { media_type, dir }`, never `dir` alone.** Every
   index entry, dependency edge, removal, and resolved target is keyed by
   `(media_type, canonical_dir)`.
2. **One write path.** All lock compilation and `state.json` writes run through
   one serialized pipeline (`run_server_update` on the daemon, `InstanceLock` in
   offline `datamile update`). The watcher resolves changed targets and submits
   requests to `run_server_update` rather than running a second compiler.
3. **Sources are not outputs.** `walk_files` skips every `*.lock.json`, lock
   freshness is tracked via `FileStat { mtime, size }`, and reading any
   `*.lock.json` through `d.fs` during compile is an error.
4. **Facts in the index, meaning in the consumer.** Shared audio URIs and CAS
   image addresses retain all owning entities in `LogicEngine`; each consumer
   queries for its own media type or CAS address.

## Allowed layouts

- One directory may hold `<media_type>.lock.json` for any number of distinct
  media types; entity ids are scoped per media type.
- Within one media type, `select` must not return two dirs where one is a
  subfolder of the other.
- Across different media types, shared dirs and nested dirs are allowed.

```text
/media/films/Inception (2010)/          allowed: two media types, nested
  movie.toml  inception.mkv  movie.lock.json
  soundtrack/
    album.toml  *.flac  album.lock.json

/media/albums/Abbey Road/               forbidden: one media type, nested
  album.toml  ...  album.lock.json
  disc2/
    album.toml  ...  album.lock.json
```

## Changes

### 1. Per-type discovery

Today `core/datamile-media/src/schema.rs` rejects both layouts:

- `claimed: HashMap<PathBuf, String>` spans all media roots, failing if two
  media types select the same directory.
- `reject_nesting` sorts all discovered dirs globally across media types.

Changes:

- Remove the cross-type `claimed` map; per-root uniqueness stays enforced by
  `select_media_roots`'s `seen` set (and within a media type if multiple roots
  share a `media_type`, check per `media_type`).
- Scope `reject_nesting` to compare adjacent dirs only within each `media_type`.
- Update `reject_nesting`'s docstring and invert the cross-type collision test in
  `core/datamile-media/src/tests.rs`.

### 2. Entity identity and update scope

Define `EntityKey` in `core/datamile-cache/src/stat.rs` and re-export it from
`datamile-cache`, `datamile-media`, and `datamile-server`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct EntityKey {
    pub media_type: String,
    /// Always canonicalized before storage or comparison.
    pub dir: PathBuf,
}
```

Changes:

- **`LogicEngine` removal (`core/datamile-logic/src/engine.rs`)**: replace
  `remove_entity_by_path(&mut self, path)` with
  `remove_entity_by_type_and_path(&mut self, media_type: &str, path: &Path)` and
  update callers in `update.rs`, `validate_and_cleanup_paths`, and
  `core/datamile-logic/src/tests.rs`.
- **Force update (`core/datamile-media/src/backend.rs`)**: `force_update_entity`
  enqueues `UpdateRequest { entities: vec![EntityKey { media_type, dir }], force: true, .. }`
  so forcing one entity never recompiles sibling media types in the same dir.
- **Update scope (`core/datamile-media/src/deps.rs`, `update.rs`, `core/datamile-server/src/backend.rs`)**:
  replace `UpdateScope` and `PendingUpdate.media_type` with:

  ```rust
  pub enum UpdateScope {
      All { media_type: Option<String> },
      Paths { paths: HashSet<PathBuf>, media_type: Option<String> },
      Entities(BTreeSet<EntityKey>),
  }
  ```

  `UpdateRequest` adds `#[serde(default)] pub entities: Vec<EntityKey>`. When
  `entities` is non-empty, `run_server_update` builds `UpdateScope::Entities`
  (canonicalizing each `dir` and ignoring `path`/`media_type`); otherwise it
  builds `UpdateScope::Paths` when `path` is set or `UpdateScope::All` when
  unset.

  Complete `UpdateScope::merge` matrix:

  | Existing \ Incoming | `All { None }` | `All { Some(b) }` | `Paths { pb, mb }` | `Entities(eb)` |
  | --- | --- | --- | --- | --- |
  | `All { None }` | `All { None }` | `All { None }` | `All { None }` | `All { None }` |
  | `All { Some(a) }` | `All { None }` | `All { Some(a) }` if `a==b` else `All { None }` | `All { Some(a) }` if `mb == Some(a)` else `All { None }` | `All { Some(a) }` if all in `eb` match `a` else `All { None }` |
  | `Paths { pa, ma }` | `All { None }` | mirror of row 2 | `Paths { pa ∪ pb, ma }` if `ma==mb` else `All { None }` | widen `All { ma }` if `ma` matches all `eb` else `All { None }` |
  | `Entities(ea)` | `All { None }` | mirror of row 2 | mirror of row 3 | `Entities(ea ∪ eb)` |

  `in_scope` checks `Entities` by exact `EntityKey` membership; the
  `"no root entity matches the given path"` bail in `update.rs` applies only to
  `UpdateScope::Paths`, not `UpdateScope::Entities` (so a vanished watcher
  target reconciles as a removal rather than failing the pass); `run_pass` and
  `run_update_pass` take `UpdateScope` directly (removing the separate
  `media_type` parameter) and claim the config digest only on
  `UpdateScope::All { media_type: None }` with zero failures.

### 3. Dependency index

`ExternalIndex` in `core/datamile-media/src/deps.rs` becomes:

```rust
pub type ExternalIndex = BTreeMap<PathBuf, BTreeSet<EntityKey>>;
```

- `index_entity(index, key: &EntityKey, external: &[PathBuf])` removes `key`
  from existing buckets (`unindex_entity(index, key)`) and inserts `key.clone()`
  for each external path.
- Note: `unindex_entity` runs once per compiled entity via `retain` on the
  external-file map (external maps remain small; if a pass compiles many
  entities, `record_state` may sweep once over the batch).
- `rebuild_external_index` reads `state.media_roots()` -> `(media_type, init)`
  and builds `EntityKey { media_type, dir }`.

### 4. Watcher and single write path

`core/datamile-media/src/watcher.rs` stops compiling, ingesting, writing state,
and broadcasting `MEDIA_UPDATED` directly:

- `MediaState::init_dirs()` (`core/datamile-media/src/state.rs`) derives watch
  dirs from `self.media_roots()` rather than `self.roots` so empty `init`
  directories at boot are watched for newly added entity dirs.
- Config dependency changes reload config and manifest, then submit
  `UpdateScope::All { media_type: None }` to `run_server_update` if needed.
- Non-config filesystem events resolve targets without holding `logic`,
  `state_writer`, or `update_lock` across `.await`:
  - External dependency hits read `ExternalIndex` directly into a
    `BTreeSet<EntityKey>`.
  - Watch-root path hits resolve affected `(media_type, init)` pairs via
    `known_root_ancestor` and `select_roots` into `EntityKey`s, or enqueue
    `UpdateScope::Paths` when an ancestor directory is deleted so missing roots
    are removed on the next pass.
- The watcher submits collected requests to `run_server_update`.
- Lock acquisition order inside `run_server_update` is strictly:
  `update_lock` (coalesce into `PendingUpdate`, max 1 pending merged slot) ->
  `spawn_blocking(run_pass)` (acquires `state_writer` in `record_state`) ->
  `logic.write()` (ingestion) -> `tx.send(MEDIA_UPDATED)`.
- `active_writes` remains keyed on `<dir>/<media_type>.lock.json` to skip
  self-triggered `inotify` events.

### 5. Playable lookups

`playable_lookup` in `core/datamile-logic/src/engine.rs` becomes:

```rust
pub playable_lookup: HashMap<String, Vec<PlayableLookupItem>>,
```

- Upserting replaces any item in the URI bucket with the same
  `(media_type, media_id)` while preserving other owners.
- Removing an entity retains only items where `(item.media_type, item.media_id)`
  differs, dropping the URI key only when the `Vec` becomes empty.

### 6. CAS image lookups

Compiled masters live in content-addressed storage at
`<cache_root>/images/<address>/master.qoi`, where `address` is the BLAKE3
digest of `(fit, width, height, filter, source_bytes)`. Because `GET /api/images/{file}`
reads `<cache_root>/images/<address>/master.qoi` directly whenever it exists,
`image_lookup` is strictly a cold-cache recovery fallback if `master.qoi` is
missing from disk, and every owner of an `address` shares identical rendering
parameters.

`image_lookup` in `core/datamile-logic/src/engine.rs` becomes:

```rust
#[derive(Clone, Debug)]
pub struct ImageSpecMeta {
    pub fit: String,
    pub filter: String,
    pub target_width: u32,
    pub target_height: u32,
}

#[derive(Clone, Debug)]
pub struct CachedImageLookup {
    pub spec: ImageSpecMeta,
    pub sources: Vec<(EntityKey, PathBuf)>,
}

pub image_lookup: HashMap<String, CachedImageLookup>,
```

- Upserting replaces an existing entry for `EntityKey` in `sources` (updating
  its path in place) or pushes `(EntityKey, abs_source_path)` if absent.
- Removing an entity removes only entries matching `EntityKey` and removes the
  `address` key only when `sources` becomes empty.
- On a cold-cache miss in `core/datamile-server/src/api/images.rs`,
  `ensure_master_image` tries the paths in `sources` in order and renders
  `master.qoi` from the first source file that exists on disk.

### 7. Freshness, single-choke lock ban, and read-only paths

- **Exclude locks from `walk_files`**: `walk_files` in
  `core/datamile-cache/src/stat.rs` skips every file whose name ends with
  `.lock.json`.
- **Track `lock_stat` inside `RootState`**:
  `RootState` gains `#[serde(default)] pub lock_stat: Option<FileStat>`.
  `CompiledRoot` gains `pub lock_stat: FileStat`, populated via
  `FileStat::from(&fs::metadata(&lock_path)?)` after writing or verifying the
  lock. `StateEntry` becomes `(PathBuf, RootFiles, FileStat)` and `run_pass`
  records `lock_stat` for every compiled root (whether `modified` is `true` or
  `false`).
- **`root_is_dirty` signature**:
  `pub fn root_is_dirty(dir: &Path, media_type: &str, walked: &BTreeMap<String, FileStat>, entry: Option<&RootState>) -> bool`
  returns `true` if `entry` is `None`, `entry.lock_stat` is `None`, `walked` or
  external files in `entry.files` changed/vanished, `<dir>/<media_type>.lock.json`
  is missing on disk, or its on-disk `FileStat` differs from `entry.lock_stat`.
- **Single choke point `CompileGuard`**:
  An RAII `CompileGuard` on `EngineContext` (`core/datamile-lua/src/lib.rs`) is
  held across `compile_root_impl` and dropped on return (including `?` exits):
  - Enforced in `EngineContext::record_dependency` (called by `parse_path_with_ctx`
    for `read`, `read_json`, `read_toml`, `read_lines`, `read_dotenv`, `exists`):
    when `CompileGuard` is active, any path ending in `.lock.json` fails with:
    `error: compile cannot read lock file '<path>'; a lock is never an input`
  - Directory enumerators (`d.fs.files`, `d.fs.dirs`, `d.fs.roots`, `d.fs.first`,
    `_scandir_native`) filter out `*.lock.json` entries while `CompileGuard` is
    active so `d.fs.files(ctx.path)` continues to work in shared directories.
  - Raw Lua `io.open` is outside `d.fs`; it is neither tracked for hot-reload
    nor guarded.
- **Read-only paths**: If `write_atomic` fails with `PermissionDenied` or
  `EROFS` on a dirty/modified lock, `compile_root_impl` fails that entity only
  with:
  `<path> could not be updated; datamile cannot recompile on a read-only path`
  and leaves its previous `RootState` entry untouched.
- **Migration**: Existing `state.json` files deserialize `lock_stat` as `None`
  and include lock paths in `files`, triggering one clean recompile on first
  `datamile update` after upgrade.
- **MPD temporary exception**: Until `datamile-mpv` replaces MPD,
  `core/datamile-server/src/mpd_bridge.rs` queries `playable_lookup` for
  `item.media_type == "album"`.

## Target resolution

`POST /api/x` delegates target resolution to one shared Rust resolver querying
the daemon's `LogicEngine` index (`MediaManifest`, `ActionMap`, and
`EntityRecord`s keyed by `(media_type, canonical_dir)`):

1. Validate `name` (canonicalized `-` -> `_`) against `ActionMap` (`404` if
   unknown).
2. Validate every entry in `media` against `MediaManifest` (`400` if
   unregistered).
3. Validate `dirs`: every entry must be an expanded absolute path; canonicalize
   each path (`std::fs::canonicalize` -> per-dir error if missing/deleted).
4. Resolve per `(canon_dir, media)` in argument order:
   - **No `dirs`, no `media`, generic action**: 1 targetless run (`media: None,
     dir: None`).
   - **No `dirs`, scoped action (or non-empty `media`)**: `400 Bad Request`.
   - **Explicit `media` provided**: literal selector of `<canon_dir>/<m>.lock.json`
     for each `m` in `media`. Succeeds if indexed in `LogicEngine`; otherwise
     sets `XRun.error` (`entity ('<m>', '<canon_dir>') is not in the entity index; run 'datamile update'`).
   - **`media` omitted, generic action (`entry.media == None`), `dirs` provided**:
     succeeds if `canon_dir` has exactly 1 indexed entity across all media
     types; otherwise sets `XRun.error`:
     - `0` entities -> `no indexed entity in '<canon_dir>'; run 'datamile update'`
     - `2+` entities -> `ambiguous target at '<canon_dir>' (<types>); pass --media`
   - **`media` omitted, scoped action (`entry.media == Some(declared)`)**:
     intersects indexed media types at `canon_dir` with `declared.keys()`:
     - `1` match -> resolves that `(canon_dir, m)` pair.
     - `2+` matches -> sets `XRun.error`: `ambiguous target at '<canon_dir>' (<matches>); pass --media`.
     - `0` matches -> if `canon_dir` has 0 indexed entities, sets `XRun.error`:
       `no indexed entity in '<canon_dir>'; run 'datamile update'`; if it has
       indexed entities outside `declared`, sets `XRun.error`:
       `no matching entity at '<canon_dir>' (indexed: <types>); pass --media to select explicitly`.
5. For each resolved `(canon_dir, m)`, read `<canon_dir>/<m>.lock.json` from
   disk (if `execute: false` or `execute: true` to populate `ctx.lock`). If the
   lock file is missing or unparseable, set `XRun.error` naming `datamile update`.

## Edge cases

| Case | Behaviour |
| --- | --- |
| Generic action, `dirs: []`, `media: []` | 1 targetless run (`lock: null, media: null`) |
| Generic action, `dirs: [X]` (1 entity), `media: []` | resolves that entity (`lock`, `media`) |
| Generic action, `dirs: [X]` (2+ entities), `media: []` | `XRun.error`: ambiguous, pass `--media` |
| Scoped (`album`), `X` has `album` + `vinyl`, `media: []` | resolves `album` |
| Scoped (`album, vinyl`), `X` has `album` + `vinyl`, `media: []` | `XRun.error`: ambiguous, pass `--media` |
| Scoped (`album`), `X` has `movie` only, `media: []` | `XRun.error`: no matching entity; pass `--media` to select explicitly |
| Any action, `dirs: [X]`, `media: [Y]` (indexed) | resolves `Y` regardless of action's declared `media` |
| Unindexed `dir` with or without `media` | `XRun.error` naming `datamile update` |
| Indexed entity whose `<dir>/<m>.lock.json` was deleted | `XRun.error` naming `datamile update` |
| Deleted or non-existent `dir` | `XRun.error` for that `dir` |
| Unregistered `media` value | `400 Bad Request` before dir resolution |
| Nested dirs inside one media type | discovery error |
| Nested dirs across distinct media types | allowed |
| Lock missing or `lock_stat` mismatch on `update` | entity marked dirty and recompiled |
| Compile reads `*.lock.json` via `d.fs` | config error via `CompileGuard` |
| `d.fs.files(ctx.path)` when lock files exist | `*.lock.json` filtered out; audio files returned |
| Read-only path with dirty entity / modified lock | entity fails with read-only error; siblings continue |
| Lock write during a pass | `active_writes` skips self-event; matching `lock_stat` stays clean |
| Same file playable under two media types | both owners kept; MPD filters `"album"` |
| Same CAS image address under two entities | shared `sources` list; cold-cache miss uses first surviving file |

## Alternatives Considered

- **Content `blake3` hashing on locks during planning**: Hashing every lock JSON
  on every `update` pass reads every lock file in the library during planning.
  `FileStat { mtime, size }` catches edits and deletions at `stat()` cost, while
  `write_atomic` prevents partial lock files.
- **Separate reverse map in `ExternalIndex`**: Storing `HashMap<EntityKey,
  Vec<PathBuf>>` duplicates `RootState.files` and adds sync overhead. `retain`
  over `BTreeMap<PathBuf, BTreeSet<EntityKey>>` keeps one source of truth.
- **Per-entity mutex between watcher and `update`**: Keeping duplicate
  compile/ingest/broadcast code in `watcher.rs` behind a per-entity mutex risks
  deadlocks with `state_writer`. Submitting watcher targets to
  `run_server_update` leaves one pipeline.

## Risks & Limits

- **Cross-root nesting `walk_files` amplification**: Recursive `walk_files` on
  an outer entity (`movie`) containing an inner entity (`album`) includes the
  inner dir's non-lock files in its `files` map, causing one parent recompile on
  inner source edits.
- **Watcher debounce window**: Between a filesystem edit and the 100ms debounce
  plus compile completing, `POST /api/x` reads the prior lock in the index.
  Callers requiring immediate freshness run `datamile update` first.
- **Compile failure isolation**: Compile or read-only errors emit
  `UpdateEvent::Failed` with `Some(dir)` and preserve the prior `RootState`
  entry so the next pass retries without crashing sibling entities or the daemon.

## Implementation

### Files

| Action | Path | Change |
| --- | --- | --- |
| Modify | `core/datamile-cache/src/stat.rs`, `lib.rs` | Define and re-export `EntityKey`; skip `*.lock.json` in `walk_files`; add `lock_stat: Option<FileStat>` to `RootState`; pass `media_type` into `root_is_dirty` |
| Modify | `core/datamile-cache/src/stat/tests.rs` | Test `walk_files` skipping `*.lock.json`, `RootState` deserializing missing `lock_stat`, and `root_is_dirty` lock checks |
| Modify | `core/datamile-logic/src/engine.rs`, `lib.rs`, `tests.rs` | `remove_entity_by_type_and_path`, multi-owner `playable_lookup` (`Vec`), CAS `image_lookup` (`CachedImageLookup`, `ImageSpecMeta`), update tests |
| Modify | `core/datamile-lua/src/lib.rs`, `functions/fs/mod.rs` | RAII `CompileGuard` on `EngineContext`; enforce `*.lock.json` ban in `record_dependency`; filter `*.lock.json` in `fs` enumerators during compile |
| Modify | `core/datamile-lua/src/functions/fs/tests.rs`, `tests/memo.rs` | Test `CompileGuard` blocking readers and filtering `d.fs.files`/`d.fs.first` |
| Modify | `core/datamile-media/src/schema.rs` | Remove cross-type `claimed` map; scope `reject_nesting` per `media_type`; update docstrings |
| Modify | `core/datamile-media/src/compiler.rs` | Hold `CompileGuard` in `compile_root_impl`; populate `CompiledRoot.lock_stat`; format read-only write errors |
| Modify | `core/datamile-media/src/state.rs` | Derive `init_dirs()` from `self.media_roots()` |
| Modify | `core/datamile-media/src/deps.rs` | `ExternalIndex` on `EntityKey`; `UpdateScope` enum + merge matrix; `StateEntry = (PathBuf, RootFiles, FileStat)` |
| Modify | `core/datamile-media/src/update.rs` | `plan_work` passes `media_type` to `root_is_dirty`; record `lock_stat` for all compiled roots; handle `UpdateScope::Entities`; call `remove_entity_by_type_and_path` |
| Modify | `core/datamile-media/src/watcher.rs` | Replace duplicate compile/ingest/broadcast pipeline with `EntityKey`/`Paths` resolution submitted to `run_server_update` without holding locks |
| Modify | `core/datamile-media/src/backend.rs`, `lib.rs` | `force_update_entity` sets `UpdateRequest.entities`; re-export `EntityKey` |
| Modify | `core/datamile-media/src/tests.rs` | Invert cross-type collision/nesting tests; add multi-lock, `lock_stat`, read-only, `CompileGuard`, and watcher-pipeline tests |
| Modify | `core/datamile-server/Cargo.toml`, `lib.rs`, `backend.rs` | Add `datamile-cache` dependency; re-export `EntityKey`; add `UpdateRequest.entities` |
| Modify | `core/datamile-server/src/mpd_bridge.rs` | Filter `playable_lookup` for `"album"` |
| Modify | `core/datamile-server/src/api/images.rs` | Try `CachedImageLookup.sources` in order on cold-cache `master.qoi` miss |
| Modify | `core/datamile-server/src/api/x.rs`, `tests.rs` | Shared target resolver backing `POST /api/x`; server tests for resolution matrix, `mpd_bridge`, and CAS `image_lookup` fallback |
| Modify | `docs/vocabulary.md`, `docs/lua-api.md` | Update §4.1, §6, §7, §8, and vocabulary after implementation |

### Phases

Each phase must be a separate commit that compiles cleanly and passes
`devenv shell -- lint` and `devenv shell -- tests`:

1. **Identity, discovery, and lookups**: `EntityKey` in `datamile-cache` (+ `datamile-server/Cargo.toml` and `lib.rs` re-exports), `remove_entity_by_type_and_path` (updating `update.rs`, `watcher.rs`, `engine.rs`, and `tests.rs`), `playable_lookup` `Vec`, CAS `CachedImageLookup` in `engine.rs` and `images.rs`, `mpd_bridge` `"album"` filter, per-type discovery in `schema.rs`.
2. **Freshness and `CompileGuard`**: `walk_files` skipping `*.lock.json`, `RootState.lock_stat`, `root_is_dirty` lock checks, `CompiledRoot.lock_stat`, `CompileGuard` in `datamile-lua` (`record_dependency` + enumerator filtering), read-only error isolation, record-on-all-compiled in `update.rs`.
3. **Single write path**: `MediaState::init_dirs()` from `media_roots()`, `ExternalIndex` on `EntityKey`, `UpdateScope` + `UpdateRequest.entities` + `force_update_entity`, route `watcher.rs` through `run_server_update`.
4. **Target resolution & docs**: Shared resolver for `POST /api/x` (paired with `specs/actions/spec.md` phases 2–3), plus `docs/vocabulary.md` and `docs/lua-api.md` updates.

### Tests

- `datamile-cache`: `walk_files` skips `*.lock.json`; `RootState` without `lock_stat` deserializes as `None`; `root_is_dirty` detects missing lock and `lock_stat` mismatch.
- `datamile-logic`: `remove_entity_by_type_and_path` leaves sibling media type in shared dir intact; multi-owner `playable_lookup` and CAS `image_lookup` survive removing one owner and replace existing `EntityKey` entries without duplicates.
- `datamile-lua`: `CompileGuard` blocks `d.fs.read_json`/`read_toml`/`read`/`exists` on `*.lock.json`, filters `*.lock.json` out of `d.fs.files`/`d.fs.first`, and drops cleanly on `?` errors.
- `datamile-media`: shared-dir and cross-type nested discovery succeed while same-type nesting fails; `ExternalIndex` preserves both `EntityKey`s on shared external edit; dirty root compiling to identical bytes updates `lock_stat`; read-only lock write fails only that entity; watcher edit submits one `run_server_update` request with the resolved `EntityKey`.
- `datamile-server`: `images.rs` recovers `master.qoi` from the second surviving owner when the first owner's file is gone; `mpd_bridge` picks `"album"` from multi-owner URI; `POST /api/x` satisfies every row of *Edge cases*.
