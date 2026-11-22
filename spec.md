# Spec: `tags` and `objects`

Status: draft, one branch (`feat/tags-objects`). This change covers `tags` and
`objects` only.

## 1. Why

The lock file currently has three homes for data: root scalar fields (`album`,
`albumartist`, `date`, `tracknumber`, …), `keys` (whatever Lua hooks returned),
and `info`/`colors`/`lyrics`. This change gives every value exactly one home:

- `tags` — flat scalar metadata: the values that look like audio file tags.
- `objects` — arbitrary JSON that has no flat form (`info`, `colors`, and
  whatever the user wants).
- `keys` disappears entirely.

Names inside `tags` and `objects` are whatever the user config chooses. Tag-like
lowercase `snake_case` is the convention, not a rule the compiler enforces.

## 2. Lock shape

### Album root

| today | after |
| --- | --- |
| `albumartist`, `album`, `date` | `tags.albumartist`, `tags.album`, `tags.date` |
| `keys.*` | `tags.*` |
| `info.*` | `objects.info.*` (built in Rust) |
| `colors` | `objects.colors` (built in Lua, like everything else) |
| `id`, `images`, `manifests` | unchanged |

### Track

| today | after |
| --- | --- |
| `discnumber`, `tracknumber`, `artist`, `title` | `tags.*` |
| `keys.*` | `tags.*` |
| `info.*` | `objects.info.*` (built in Rust) |
| `lyrics = { type, text }` | `tags.lyrics` = raw text |
| `file` | unchanged |

`verified` does not exist yet; when the `history.toml` divergence check is
implemented it lands in `objects.verified` and stays a boolean.

### Result

```jsonc
{
  "album": {
    "id": "Autechre - Confield",
    "tags": {
      "album": "Confield",
      "albumartist": "Autechre",
      "date": "2001-04-30",
      "genre": ["Electronic", "Abstract"]
    },
    "objects": {
      "info": {
        "virtual": false,
        "total_discs": 1,
        "total_tracks": 9,
        "duration_milliseconds": 3012000,
        "duration_formatted": "50:12"
      },
      "colors": { "foreground": "#ffffff", "background": ["#1a1a1a"] }
    },
    "images": { "main": [ /* … */ ] },
    "manifests": { /* … */ }
  },
  "tracks": [
    {
      "tags": { "tracknumber": 1, "discnumber": 1, "title": "VI Scose Poise", "artist": "Autechre" },
      "objects": {
        "info": {
          "sample_rate": 44100, "bits_per_sample": 16, "bitrate_kbps": 900,
          "encoding": "flac", "channels": 2,
          "duration_milliseconds": 401000, "duration_formatted": "6:41"
        }
      },
      "file": { "path": "01 - VI Scose Poise.flac", "mtime": 0, "byte_size": 0 }
    }
  ]
}
```

Keys inside `tags` and `objects` are sorted (`sort_json_keys`, as today).

## 3. Lua API

```lua
datamile.compile.album.tag(name, fn | true)   -- -> album.tags[name]
datamile.compile.album.object(name, fn)       -- -> album.objects[name]
datamile.compile.track.tag(name, fn | true)   -- -> track.tags[name]
datamile.compile.track.object(name, fn)       -- -> track.objects[name]
```

`compile.a` / `compile.track` / `compile.t` aliases keep pointing at the same
tables.

Semantics:

- `true` shorthand reads the manifest, same as `compile.*.key(name, true)` does
  today: `m.metadata.album[name]` for album, the track entry for tracks.
- Return value `nil`, `""`, or `{}` → the name is omitted from the lock. Nothing
  is written as `null` and no empty container is emitted.
- Tag values: string, number, boolean, or a list of those. Anything else is a
  compilation error (see §5).
- Object values: any JSON-representable value (nested tables with string keys,
  lists, scalars). Values that cannot be serialized to JSON are a compilation
  error.
- Names are arbitrary strings chosen by the user: no case, charset, or
  reserved-name checks.
- Registration order does not matter; the lock output is sorted (§4).
- The user's names collide freely with the Rust-provided defaults: a Lua
  `object("info", …)` replaces `objects.info` wholesale. Rust only fills a
  default when Lua produced nothing for that name.

### Defaults shipped in the binary

`LuaEngine::new()` loads `compiler.lua` before the user's `init.lua`, so defaults
are ordinary registrations that user config overrides by registering the same
name again (`REGISTRY` slot is overwritten).

Album tags:

| tag | value |
| --- | --- |
| `albumartist` | `[album].albumartist` → `[album].artist` |
| `album` | `[album].album` |
| `date` | `[album].date` |

Track tags:

| tag | value |
| --- | --- |
| `tracknumber` | entry `tracknumber` |
| `discnumber` | entry `discnumber` |
| `title` | entry `title` |
| `artist` | entry `artist` → album `albumartist` (preserves today's Rust fallback) |

`info` is a Rust-provided default, not a Lua rule, so a bare config still
produces usable locks. It is also the only `objects` name Rust claims, and only
until the user config registers the same name itself:

- album `objects.info` = `virtual`, `total_discs`, `total_tracks`,
  `duration_milliseconds`, `duration_formatted` (all already computed in
  `compile/build.rs`).
- track `objects.info` = `sample_rate`, `bits_per_sample`, `bitrate_kbps`,
  `encoding`, `channels`, `duration_milliseconds`, `duration_formatted` (from
  `ctx_tracks`).

`colors` and everything else come from the user's Lua config.

### Colors are not validated

Nothing checks hex strings anymore. `validate_and_format_colors` and
`is_valid_hex_color` are deleted, along with `DatamileError::InvalidColorFormat`;
`build.rs` stops reading `theme.toml`. A malformed color reaches the lock as
written by config, and the shader falls back to its own palette at render time.

Deleting the Rust helper also deletes its two side effects, so config owns the
shape too:

- no key filtering — whatever config puts under `colors` is written verbatim;
- no normalization — `background` must come out as a list of strings and
  `foreground` as a string, because the web app indexes `background` as a list
  (`BackgroundShader.svelte`) and reads `foreground` as a scalar.

## 4. Determinism

The lock is compared byte-for-byte with the existing file before writing
(`compile/stream.rs`) and is the only input to change detection and to the
server. Same inputs must produce the same bytes.

- `tags` and `objects` keys are sorted by raw byte order (ASCII), recursively
  through `objects`, by the existing `sort_json_keys` before the album object is
  turned into the lock. Nothing else reorders anything.
- Hook evaluation order cannot leak into the output. The dispatcher walks
  `REGISTRY` with `pairs`, whose order depends on the Lua hash seed and differs
  per process, but every result is keyed by name, never appended.
- Registration order in `init.lua` is irrelevant for the same reason: two
  configs registering the same set of tags produce identical locks.
- Tag lists keep the order the hook returned. They are not sorted — order is
  data (`genre` first entry, primary `styles`), and sorting would rewrite user
  intent.
- Where the Rust default and a Lua registration share a name, Lua wins by one
  rule applied in one place: Rust fills `info` only if the Lua result has no
  `info`. The outcome never depends on which side enumerates first.
- Tracks stay in manifest order (disc, track), as today.
- Numbers stay numbers (`tracknumber` is `1`, not `"1"`) and booleans stay
  booleans; serde_json's number formatting is the only formatting applied.
- Consequence: when several hooks fail in one album, which error is reported
  first is not guaranteed, because `pairs` decides. Each message names its tag,
  so it stays actionable.

## 5. Validation

One check, applied in the dispatcher loop for both album and track tags:

- scalar (`string`/`number`/`boolean`) → accepted.
- table → every element must be a scalar and the table must be a pure array;
  otherwise error.
- anything else (`function`, `userdata`, nested table as an element) → error.

Error text names the tag and its owner, and is raised through `error()`, which
the Rust side already wraps in `ManifestParseError` with the album path:

```
Error evaluating album tag 'styles': expected scalar or list of scalars, got table-of-table
Error evaluating track tag 'lyrics' at index 3: expected scalar or list of scalars, got function
```

`false` is a legal tag value (the UI uses `tags.instrumental == true`); it is not
an error. Only non-scalar values are.

## 6. Code changes

### Lua (`core/libdatamile/src/lua/`)

- `core.lua`: `REGISTRY.keys` → `REGISTRY.tags`; add `REGISTRY.objects = { album = {}, tracks = {} }`; drop `REGISTRY.lyrics`.
- `compiler.lua`:
  - `compile.album.key` → `.tag`, `compile.tracks.key` → `.tag`;
  - add `.object` on both;
  - delete `compile.tracks.lyrics` — `lyrics` is an ordinary track tag now;
  - add the default tag registrations (§3);
  - one shared `set_tag(registry, owner, name, fn)` helper that also does the
    type check, used by both album and track loops;
  - dispatcher returns `{ id, tags, objects, images }` for the album,
    `{ tags, objects }` per track; result maps only get a key when the hook
    returned something non-empty.

### Rust engine (`core/datamile/src/compile/`)

- `build.rs`: `assemble_album_object` inserts `tags` and `objects` instead of the
  four root scalars and `keys`; `AlbumHeader` drops `artist`/`title`/`date`
  (keeping `id`); `parse_mandatory_album_fields` stays as validation only
  (`Result<()>`) so an album without `[album].album`/`albumartist`/`date` still
  fails to compile.
- `tracks.rs`: `build_final_tracks` emits `{ tags, objects, file }`; drops the
  `discnumber`/`tracknumber`/`artist`/`title` inserts and the lyrics
  extract-and-move hack; builds `objects.info` from `ctx_tracks`.
- `utils.rs`: `sort_json_keys` stays; `validate_and_format_colors` and
  `is_valid_hex_color` are deleted with their call site in `build.rs`.
- `libdatamile/src/error.rs`: `InvalidColorFormat` variant removed.

### Rust server (`core/datamile/src/server/`)

- `logic/mod.rs`: `dictEntry` becomes `{ id, tags, objects, images }` — the flat
  `album`/`albumartist`/`date`/`duration_*`/`total_*`/`virtual`/`colors` fields
  are dropped, since they were only duplicates of `tags`/`objects.info`. Album
  `title`/`artist` for the record come from `tags`. Track ingest reads
  `track.tags.{tracknumber,discnumber,title,artist}` and `track.objects.info`.
- `api/playback.rs`: disc filter reads `track.tags.discnumber`.

### Web app (`interfaces/web-app/src/`)

- `types.ts`: `AlbumLockData` = `{ id, tags, objects, images, manifests }`;
  `TrackItem` = `{ id?, tags, objects, file }`; drop `TrackLyrics`,
  `AlbumInfoData`, `TrackPhysicsInfo` move under `objects`.
  `AlbumCardItem`/`AlbumDictEntry` keep their flat UI shape (`title`, `artist`,
  `total_discs`, `duration_formatted`, `virtual`, `colors`) — it is a UI
  projection derived in `mapIdsToAlbums`.
- `collection.svelte.ts` / `view.svelte.ts`: derive card fields from
  `dict.tags` / `dict.objects.info` / `dict.objects.colors`.
- `QueueView.svelte`: `album.objects.colors.background|foreground`.
- Track consumers (`TracklistView`, `ModalDrawer`, `ModalDrawerTracks`,
  `TrackPanel`): `track.tags.title|artist|tracknumber`, `track.objects.info.*`.
- `LyricsView`: `track.tags.instrumental === true`, `track.tags.lyrics` string.

### Docs

- `README.md`: `compile.album.key` → `compile.album.tag`, lock shape snippet.

## 7. Migration

1. User config (lives outside this repo): `.key` → `.tag`, `a.keys.x` →
   `a.tags.x`, `a.info` → `a.objects.info`, `a.colors` → `a.objects.colors`,
   `compile.tracks.lyrics` → `compile.track.tag("lyrics", …)` returning a string.
2. One forced full recompile (`datamile update --force`). Existing locks hold
   `keys`; new server code reads `tags`, so stale locks render "Unknown".
3. Add `env!("CARGO_PKG_VERSION")` to `update/cache.rs::lua_config_hash`. The
   shipped defaults live in `compiler.lua`, which is loaded via
   `lua.load(include_str!(…))` and is therefore *not* in `REGISTRY.dependencies` —
   without the version salt, a binary upgrade never invalidates old locks.

## 8. Tests

New `core/libdatamile/src/lua/tests/dispatcher.rs` (Rust, drives `LuaEngine` and
`__DATAMILE_DISPATCHER` directly, no filesystem):

- tag returning a scalar lands in `tags`;
- tag returning a list of scalars lands in `tags` verbatim;
- tag returning a nested table errors, message contains the tag name and the
  album/track owner;
- tag returning `nil`, `""`, `{}` leaves the name out of `tags`;
- object returning a nested table lands in `objects` verbatim;
- `false` is accepted as a tag value.

Manual check: compile one physical and one virtual album, diff the lock against
the expected shape from §2.

## 9. Out of scope

- `musicbrainz_*`-driven UI features — theoretical for now.
- `location` / `files` / per-file `mtime` restructure from the docs.
- `objects.verified` implementation.
- Any color validation or fallback: config writes `objects.colors` unchecked, the
  shader already handles an empty palette.
- Any tag name validation or casing rules.
