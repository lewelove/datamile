# Datamile Milestones

Future work only. Implemented design is the code; `notes/docs/` was deleted because
the generic media model superseded it.

Reference points for current state:

- Media compile: `core/datamile/src/compile/media.rs`, `core/libdatamile/src/lua/compiler.lua`
- Lock schema: `{ id, tags, objects, images, location, <children>[] }`
- Logic: `core/datamile/src/server/logic/{mod,query}.rs`
- Covers: `core/datamile/src/compile/images.rs`, `core/datamile/src/server/api/assets.rs`

---

## 1. MPD API Standardization

Every MPD-facing route moves under `/api/mpd/`. The current flat namespace
(`/api/play`, `/api/queue`, `/api/toggle-pause`, ...) is a v0 artifact.

```
/api/mpd/albums/{id}                     GET     lock payload
/api/mpd/albums/{id}/play                POST    replace queue, play from 0
/api/mpd/albums/{id}/queue               POST    {"position": "tail"|"next"}
/api/mpd/albums/{id}/discs/{n}/play      POST    disc only
/api/mpd/albums/{id}/discs/{n}/queue     POST    {"position": "tail"|"next"}
/api/mpd/control/{action}                POST    stateless toggles, see below
```

Control actions:

- `toggle_pause`, `toggle_repeat`, `toggle_random`, `toggle_oneshot`, `toggle_consume`
- `toggle_single` — cycles single <-> normal
- `volume_delta` — `-100..=100`, normalized
- `seek_delta` — milliseconds, normalized

Bare `/api/mpd/` returns the INIT dict.

Unchanged and out of scope: `/api/state`, `/api/internal/*`, `/ws`, `/api/actions/{name}`.

---

## 2. Library / View API

Clients currently compose views through `/api/internal/query`, which is not a
public contract. Two endpoints replace it:

```
GET /api/library/      all declared primitives + actions + interfaces + labels
GET /api/view          albums for one library + optional filter + order + grouper
GET /api/album/{id}    single lock                        (already exists)
```

`GET /api/view` params: `library`, `filter`, `order`, `grouper`, `group`.
Response: ordered `album.id` array, matching `LogicEngine::request_view`.

`GET /api/library/` payload:

- libraries: labels + which filters/groupers/orders are visible in them
- cabinets: shelves + orders
- shelves: labels
- filters / groupers / orders: labels + types
- actions / interfaces: labels + descriptions

---

## 3. Full-Text Search

Replace the ad-hoc `search` key in `logic/query.rs` (substring over title +
artist) with a real index.

- Index is built in memory at config-eval time from scalars only.
- Every registered `media.type(...).tag(key, fn)` becomes an index field
  automatically; `datamile.search(name, fn)` overrides or adds fields.
- A field returning non-scalar is a config evaluation error.
- `GET /api/search?query=` takes a URI-encoded query string, returns `album.id[]`.
- Syntax error returns an error body; clients freeze the view until a successful
  response arrives.

Decision pending: Tantivy versus a hand-rolled inverted index. Tantivy is a large
dependency for a library of ~10^3 albums; measure before committing.

---

## 4. Cover Cache Completion

Already live: master generation, derivative rendering, prewarm endpoint,
`{size, filter, quality}` cache keys, address = content+params hash.
Remaining:

- Rename `/api/images/{file}` -> `/api/covers/{address}.{format}`.
- Rename cache root `{cache_dir}/images/` -> `{cache_dir}/covers/`.
- Document and freeze the address preimage:
  `BLAKE3(fit_mode ‖ 0x00 ‖ width_LE ‖ height_LE ‖ filter ‖ 0x00 ‖ source_hash)[..16]`,
  base64url no-pad.
- Two-tier GC:
  - Tier 1, on every update: delete `{cache_dir}/covers/{address}/` trees whose
    address no longer belongs to any in-memory lock.
  - Tier 2, derivative TTL: touch mtime on every derivative served; prune
    derivatives older than `cover_cache_ttl_days` (default 3). `master.qoi` is
    exempt and owned by tier 1.
- Verify `ETag` / `If-None-Match` / `Cache-Control: immutable` on the serve path.

---

## 5. Seeded Random Order

Deterministic shuffle as a built-in order:

```
keys  — list of lock paths, defaults to ["id"]
seed  — arbitrary string, defaults to ""
```

Order = radix sort by `BLAKE3(seed ‖ key values)`. Same keys + same seed gives
the same view on every reload. Implement as a Lua built-in order using
`d.fn.hash_string`, not Rust.

---

## 6. Actions / `datamile x`

- `--debug` is parsed and discarded (`_debug` in `x::execute`). Wire it up:
  silence action noise by default, inherit stdout/stderr when set.
- `datamile x` hands the terminal to the child process, matching
  `datamile interface`.
- Rework `musicbrainz_search` out of `actions/rework_needed/`: build
  `artist:{albumartist} AND releasegroup:{album}` from a lock or a Discogs URL,
  then `xdg-open` the MB advanced search.

## 7. Interface Polish

- Derive every white/neutral from one OKLCH white via alpha instead of literal
  per-element colors.
- Add a last-played album object so QueueView always has something to render
  when MPD is idle.
- Drop inactive elements from the queue sidebar.

---

## Rejected

- `system.toml` / `date_generated` — `local` and `history` manifests cover it.
- `datamile.compile.album.key` / `.tracks.key` docs — superseded by
  `media.type(...).tag()` / `.object()`.
- SQL filter expressions (`sql.where = "..."`) — filters are Lua predicates.
- Addressable `Set` / `Sequence` / `Facet` primitives with BLAKE3 addresses —
  `LogicEngine` caches bitmaps already; addresses add nothing over cache keys.
- `/api/scripts/{name}` — `/api/actions/{name}` is the same thing.
- `/api/config/files/{name}` — interfaces are separate processes and serve
  their own assets.
- Lua-driven manifest engine — media `input` callbacks already read sidecars;
  no use case that plaintext manifests + Rust manifest do not cover.
- `dl.*` / `vl.*` config namespace — now `datamile.*` (`d.*`).
- `metadata.toml` — renamed to `album.toml`.
- `cover-palette` action wrapper — the `theme` action already emits palettes.
