# Multi-lock entity dirs

Status: plan. Not approved, not implemented. Records the 2026-11-24 decision
that one directory may hold several media types' locks, and what that costs
in discovery, indexing, lookups and lock lifecycle.

`specs/actions/spec.md` holds the invocation rules that depend on this
(`Invocation` and the action guard under it); everything below is this
document's own subject.

## Core design rules

Four rules replace the old `1 dir == 1 lock` assumption throughout:

1. **Identity is `(media_type, dir)`, never `dir` alone.** A directory is raw
   material; a media type is a lens over it. Every index, watcher target,
   dependency edge, removal, and update scope carries `RootKey { media_type, dir }`.
   No function in the engine removes, updates, or indexes an entity by bare path.
2. **One writer, one door.** All compilation and state writes flow through one
   serialized pipeline (`run_server_update` when the daemon is live, `InstanceLock`
   when a CLI runs offline). The watcher translates filesystem events into
   scoped update requests instead of running a second compiler in parallel.
3. **Strict wall between ingredients and baked output.** Source files are
   ingredients; `*.lock.json` files are baked outputs. Fingerprinting skips
   every lock file, lock freshness is checked by checksum recorded from the
   written bytes, and compiling may never read any lock file.
4. **Facts in the index, meaning in the consumer.** Shared audio files and image
   addresses keep all owners in the index; each consumer (such as MPD) asks for
   the media type it owns.

## Intent

The same files can be several media types at once. One folder of FLACs is a
valid `album` (generic playback shape), a valid `vinyl` (`sides` node holding
`tracks`), and a valid `cd` (CD-only metadata). These are different
compilations of the same audio, not different audio.

```text
/media/abbey-road/
  album.toml  cd.toml  vinyl.toml
  01 - Come Together.flac   02 - Something.flac   ...
  album.lock.json  cd.lock.json  vinyl.lock.json
```

The alternative is worse both ways: one media type holding all three shapes
means a bloated lock whose nodes mix vinyl sides, CD offsets and a plain track
list; one folder per shape means entity dirs whose files live outside them, and
every `--dir`, lock and entity path pointing at a folder that holds no media of
its own.

## The rule

- A dir may hold `<type>.lock.json` for any number of media types. Each lock is
  the entity of its own media type; ids are per media type and need not agree.
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

`schema.rs` refuses both shapes today:

- `claimed: HashMap<PathBuf, String>` spans the whole pass, so two roots that
  select the same dir collide even when their media types differ
  (`root directory X is claimed by both 'album' and 'movie'`).
- `reject_nesting` compares every selected dir with every other, across media
  types. Its docstring justifies this with "so every file belongs to at most
  one of them"; that guarantee is what this plan gives up.

Both checks scope to one media type: uniqueness is checked per `(media_type, dir)`
(already enforced per root by `seen`), and `reject_nesting` compares dirs only
within each media type. The tests at `tests.rs` and the docstring at `schema.rs`
change with it.

### 2. Root identity & entity index

`RootKey { media_type: String, dir: PathBuf }` becomes the universal entity handle:

- **`LogicEngine` removal (`engine.rs`)**: `remove_entity_by_path(&mut self, path)`
  is replaced by `remove_entity_by_type_and_path(&mut self, media_type: &str, path: &Path)`
  so `update.rs`, `watcher.rs`, and `validate_and_cleanup_paths` never delete
  a sibling media type sharing the same dir.
- **Force update (`backend.rs`)**: `force_update_entity` passes
  `media_type: Some(media_type.to_string())` in `UpdateRequest` so
  `POST /api/media/{media_type}/update/{*id}` recompiles only that media type.
- **Update scope (`update.rs`, `deps.rs`)**: `UpdateScope` adds
  `Roots(HashSet<RootKey>)` alongside `All` and `Paths(HashSet<PathBuf>)` so
  watcher-resolved targets name exact `(media_type, dir)` pairs without pulling
  in sibling types or parent roots in nested layouts.

### 3. Dependency index

`deps.rs` is `BTreeMap<PathBuf /*external file*/, BTreeSet<PathBuf /*dir*/>>`
and `index_root` begins with `unindex_root(dir)`, which drops every entry for
that dir. With two media types in one dir, registering the second erases the
first, and an edit to a shared file recompiles one type only.

`RootKey` replaces `dir`, and a reverse map makes removal cost only that root's
own files:

```rust
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, Clone, Debug)]
pub struct RootKey {
    pub media_type: String,
    pub dir: PathBuf,
}

pub struct ExternalIndex {
    pub forward: BTreeMap<PathBuf, Vec<RootKey>>, // external file -> readers
    pub reverse: HashMap<RootKey, Vec<PathBuf>>,  // reader -> its own files
}
```

```text
/media/abbey-road/cover.jpg read by album.toml and cd.toml

forward: cover.jpg        -> [album/abbey-road, cd/abbey-road]
reverse: album/abbey-road -> [cover.jpg]
         cd/abbey-road    -> [cover.jpg]
```

- `index_root(key, external)`: unindex `key`, then write both maps.
- `unindex_root(key)`: pop `reverse[key]`, remove `key` from each bucket, drop
  empty buckets. Today's version is a `retain` over the whole index.
- Boot rebuild needs nothing new on disk: `rebuild_external_index` already walks
  `state.media_roots()` -> `(media_type, init)`.

### 4. Watcher & single compile pipeline

Today `watcher.rs` runs a second compiler/ingestor/state-writer independently of
`run_server_update` (`update.rs`), which races with startup auto-update and CLI
updates, duplicates `ingest_outcome` / `record_state` / `broadcast`, and has
three multi-lock bugs (`removed_roots` matching on bare `dir`, `state.roots`
filtering out runtime-added roots in `collect_external_dependencies`, and no
per-entity serialization).

Under rule 2 (**One writer, one door**):

- Config changes reload config + manifest as today, then trigger a scoped/full
  pass if needed.
- Non-config filesystem events resolve affected `RootKey`s:
  - External dependency hits read `ExternalIndex.forward` directly (`RootKey`
    requires no lookup in the boot-time `state.roots` list).
  - Watch-root path hits scan via `select_roots` / `known_root_ancestor` per
    `(media_type, init)` to collect affected `RootKey`s (plus full pass on
    deletions or `UpdateScope::Roots` with discovery).
- The watcher submits those targets into `run_server_update` (`UpdateScope::Roots`)
  instead of compiling or writing state directly. All locking, compilation,
  ingestion, `ExternalIndex` updates, `state.json` writes, and `MEDIA_UPDATED`
  broadcasts happen in that one pipeline.
- `active_writes` already keys on the lock file path (`<dir>/<media_type>.lock.json`)
  and stays strictly as a latency optimization to skip waking the watcher on
  self-written locks.

### 5. Playable lookups

`playable_lookup` (`engine.rs`) is URI -> one `PlayableLookupItem` carrying
`media_type`, `media_id`, `node_path`. Plain `insert` on upsert and plain
`remove(uri)` on delete mean three views of one file produce three inserts and
one winner, and deleting one entity deletes an entry another still needs.

```text
05 - Money.flac playable under album and vinyl
  upsert album, then vinyl      -> both owners kept
  track_meta asks "album"       -> one stable answer
  unindex vinyl                 -> album's entry survives
```

`playable_lookup` keeps every owner per URI (keyed by `(media_type, media_id)`
within the URI bucket, or `HashMap<String, Vec<PlayableLookupItem>>`). Removal
drops only entries matching the removed `(media_type, media_id)`. Which owner a
consumer picks is not `datamile-logic`'s decision (see *Consumers*).

### 6. Image lookups

`ImageLookupItem` has no media type or id, only `entity_dir`, so `image_lookup`
needs shared ownership keyed by `(media_type, media_id)`—a map or list of
`(OwnerKey, ImageLookupItem)` per `address`—so unindexing one entity cannot
orphan another's image address (and if two dirs hold identical image bytes,
removing one dir still leaves a valid `entity_dir` from the surviving owner).

## Freshness and lock lifecycle

The lock is a compiled output that outlives its entity (`lua-api.md` §7). Both
facts matter: freshness is decided against the entity's inputs, and an edit to
the output is breakage, not data.

```text
state entry for album/abbey-road
  inputs: { album.toml: {mtime,size}, 01 - Come Together.flac: {mtime,size},
            /media/abbey-road/cover.jpg: {mtime,size} }
  lock:   blake3("album.lock.json") = 9f2c...
```

### No lock is an input

`walk_files` skips every `*.lock.json` in the dir. Otherwise the compiled output
becomes an input and every compile dirties its own entity, and in a multi-lock
dir each type would depend on its neighbour's lock—a recompile ping-pong with
no end.

Reading any lock during a compile is an error, below.

### Any deviation in the lock means recompile

```text
missing                        -> compile
unparseable (partial write)    -> compile
digest differs                 -> compile   (a hand edit is breakage)
inputs changed                 -> compile   (existing dirty rule)
nothing                        -> use the lock as-is
```

`RootState` gains `#[serde(default)] pub lock: Option<String>`. During planning,
the lock file on disk must exist, parse as JSON, and match `entry.lock`'s blake3
digest.

A lock has no version key. Its only top-level key is the media type, so
`lock[media]` is the entity, and a shape change is handled by recompiling
rather than by reading a version number.

### The digest is recorded from the bytes just written

`CompiledRoot` carries `lock_hash: String` computed directly from the exact
`lock_json` bytes produced/written (never from a later stat), and `record_state`
persists that digest inside the single writer's pipeline. That is what makes the
loop terminate: after a compile, `digest(lock on disk) == digest(recorded)`, so
the watcher-triggered pass finds the entity clean and stops after one pass.

Three conditions, all required:

1. Digest recorded from the written bytes, in the same pipeline step as the write.
2. No lock file is an input (the fingerprint walk skips `*.lock.json`).
3. One writer per entity (enforced by routing all updates through one pipeline).

### Locks are never deleted, never renamed

A lock left behind by a dropped entity stays under its normal name. The state
entry and the entity's fingerprints decide liveness, so the leftover is inert:

```text
/media/gone/album.lock.json        album.toml deleted, lock left behind
  x --dir /media/gone --media album  -> error: not in entity index; run datamile update
  x --dir /media/gone                -> error: media type not guessable
```

The invariant "a library dir holds exactly the locks of the media types that
claim it" is deliberately not enforced on disk. Pruning it would turn a
discovery regression into data loss: a root whose `select` temporarily returns
nothing would take every lock of its type with it, and the lock is the only
record of the previous compile.

### All lock reads during compile are an error

```text
/media/boxset/package.toml
  tags = { tracks = d.fs.read_json("../disc1/album.lock.json").tags.tracks }

error: boxset reads disc1/album.lock.json; a lock is never an input.
       Read disc1/album.toml or the source files instead.
```

Enforced in `EngineContext` (`datamile-lua`): while `compile_root` is active,
`EngineContext` marks `compiling = true` (or holds the active `RootKey`), and
any `*.lock.json` path passed to `d.fs` (`read`, `read_json`, `read_toml`,
`read_lines`, `read_dotenv`, `exists`)—including the entity's own lock, so a
compile can never depend on its previous output—fails as a config error.
(`d.json` has only `encode`/`decode` and does not touch paths.)

Three failures this closes:

- `compile_roots` runs dirty entities in parallel with no ordering, so `boxset`
  would read `disc1`'s old lock while `disc1` writes a new one, and the state
  entries—computed after the whole phase—would record the new lock's stat
  while `boxset`'s content came from the old one. Clean-looking and silently
  wrong, forever.
- A filename filter cannot distinguish "my output" from "your input", so the
  exclusion rule above would erase a legitimate dependency.
- Mutual lock reads and self-lock reads have no termination or idempotency
  guarantee on the per-entity path.

Reading member manifests instead gives the same invalidation through ordinary
external dependencies, which the dep index and watcher already carry.

### Read-only paths

An edited lock means recompile, and recompile needs a write. On a read-only
mount, `PermissionDenied` / `ReadOnlyFilesystem` on lock write is a loud
failure naming the lock path, not a silent run on broken data:

```text
album.lock.json could not be updated; datamile cannot recompile on a read-only path
```

### Migration

State files written before this change have no `lock` field (`Option<String>`
defaults to `None`). Absence is treated as a mismatch: one full recompile on
the first pass, then stable. The fingerprint change (locks excluded) has the
same one-time cost, so both land together.

## Consumers

The Rust binary is media agnostic apart from MPD, which owns the
`album (root) -> track (node)` shape only. So:

- `playable_lookup` holds all owners per URI, and `mpd_bridge` / `playing_target`
  ask for `"album"` instead of taking whatever won the map. No precedence rule,
  no new config surface: the consumer names the media type it owns, the same
  way an action declares its media.
- A future `datamile-mpv` crate will own sessions, and each session owns its
  entities and reports back to the client. That is where a second playback shape
  resolves its own URIs.

## Docs and invariants

- `philosophy.md` and `schema.rs` state the single-owner guarantee this plan
  replaces. A file now belongs to one entity per media type that claims it.
- `lua-api.md` §4.1, §6 and §7 describe dir uniqueness and lock liveness; they
  need the multi-lock reading.
- `docs/vocabulary.md` carries **media entity**, **entity dir**, **entity
  index**, **stale** and **state entry**.
- `specs/actions/spec.md` carries the action-specific guard: a pair whose media
  the action does not declare is an error.

## Target resolution

`datamile x` targets entity dirs and guards them with media. This section owns
the rules that were previously split between the two documents.

- `--dir <entity-dir>...` is repeatable. When omitted for a **scoped** action
  (or when `--media` is passed), it defaults to `.`. When omitted alongside
  `--media` for a **generic** action (`media = nil`), the action runs once
  with `ctx.lock = nil` and `media = nil` (no folder target).
- `--dir` is the entity dir, not a media root's `init`. There is no subdirectory
  search and no walking up.
- `--media <media type>...` is optional and repeatable. A value that is not
  a registered media type is an error before any dir is resolved.
- Runs are the cross product of `--dir` and `--media`, in argument order.
- One result per run. Failures are collected, and the command exits non-zero
  when any run failed. An absent entity is a failure, never a skip.
- `x` never compiles and never checks staleness. It reads the entity index,
  which is the truth, because the watcher keeps it current. A dir and media
  pair the index does not hold is an error that names `datamile update`.
- When `--media` is absent (and a dir is being resolved), the dir must resolve
  to exactly one entity.
- The action's declared media is a guard, never a hint. A pair whose media
  the action does not declare is an error for that pair.

The client path reaches the same answer: an entity's `location.path` is the
canonical absolute entity dir, so a client passes it as `--dir` together with
the media type it renders.

## Edge cases

| Case | Behaviour |
| --- | --- |
| generic action, no `--dir` and no `--media` | runs once with `ctx.lock = nil` |
| dir with 2+ entities, no `--media` | error: pass `--media` |
| dir with 1 entity, no `--media` | runs it |
| unknown dir with `--media` | error naming `datamile update` |
| unknown dir, no `--media` | error: media type not guessable |
| dir deleted | error for that dir |
| unregistered `--media` value | error before any dir is resolved |
| action does not declare the pair's media | error for that pair, the rest still run |
| nested dirs inside one media root | discovery error, unchanged |
| nested dirs across media roots | allowed |
| hand-edited lock | recompiled, edit discarded |
| lock missing, unparseable or hand-edited | recompiled |
| compile reads any `*.lock.json` | config error |
| state file from before digests existed | one full recompile, then stable |
| read-only library with a modified lock | loud error: cannot recompile here |
| lock write during a pass | watcher wakes up, sees matching digest, does nothing |
| same file playable under two types | both owners kept; the consumer picks |
| same image address under two entities | shared ownership; removing one keeps the other |

## Non-goals

- Playback sessions and the `datamile-mpv` crate.
- Lock-as-input: an entity whose tags come from another entity's lock needs a
  build graph with wave recompilation and cycle rejection. Separate document.
- Pruning dead locks (see *Locks are never deleted*).
- Making `actions` media agnostic beyond what `specs/actions/spec.md` records.

