# Spec: `d.fs.roots(dir, marker, opts?)` & Strict Opt-In `prune`

## Status

Draft

## Context & Problem

`d.fs.roots` (`core/datamile-lua/src/functions/fs/mod.rs`, `core/datamile-lua/src/functions/fs/mod.lua`) currently walks the entire directory tree below `dir` without stopping when a directory matches a marker.

For an entity directory such as `Artist - Album/{album.toml, Info/, Lyrics/, Digital Covers/, Disc 1/, Disc 2/}`, `fs_roots` continues descending into every subdirectory and checks every marker against each of them. The same happens in `watcher.rs` (`handle_known_root`) whenever a known entity dir is verified on hot reload.

In addition, the current `d.fs.roots` implementation has loose input handling:
- It accepts a second-argument table wrapper `{ markers = { ... } }` with fallback `marker.markers or marker`, which silently turns key typos (like `{ marker = { ... } }`) into an empty marker list `{}`.
- It accepts empty strings (`""`), `"."`, `".."`, and empty lists (`{}`), which either match every directory on disk (`path.join("")` is `path` itself) or silently return `{}`.

## Goals

- Standardize the signature to `d.fs.roots(dir, marker, opts?)` with no legacy `{ markers = ... }` wrapper.
- Strictly validate `dir`, `marker`, and `opts`, failing loudly on empty markers, non-string markers, array/table shape mistakes, unknown option keys, or non-boolean option values.
- Add strict opt-in `opts.prune` (`boolean`, default `false`) to stop descending into subdirectories of any matched directory.

## Non-Goals

- Making `prune = true` the default (`reject_nesting` in `core/datamile-media/src/schema.rs` still detects accidental nested same-type entity dirs when `prune` is `false`).
- Replacing `WalkDir` with `read_dir` in-memory matching or parallel traversal (tracked separately in Bug Reports B and C).

## API Contract

### Signature (`core/datamile-lua/src/functions/fs/mod.lua`)

```lua
d.fs.roots(dir, marker, opts?) -> string[]
```

Examples:

```lua
d.fs.roots(dir, "album.toml")
d.fs.roots(dir, { "album.toml", "virtual.toml" })
d.fs.roots(dir, "album.toml", { prune = true })
d.fs.roots(dir, { "album.toml", "virtual.toml" }, { prune = true })
```

### Input Validation (Lua layer)

All validation errors raise a Lua `error(...)` immediately before calling native Rust:

1. **`dir`:**
   - Must satisfy `type(dir) == "string" and dir ~= ""`.
2. **`marker`:**
   - Must be either a `string` or a sequential array `table` of strings.
   - If `string`: must be a valid marker (`validate_marker`).
   - If `table`:
     - Must have `#marker > 0`.
     - Every key in `pairs(marker)` must be an integer `i` with `1 <= i <= #marker` (rejects `{ markers = { ... } }`, `{ prune = true }`, sparse tables, and mixed dict/array tables).
     - Every `marker[i]` must satisfy `validate_marker`.
     - Deduplicate markers while preserving order.
   - **`validate_marker(m)`:**
     - Must be a `string`.
     - Must not be `""`, `"."`, or `".."` (all three match every directory unconditionally via `path.join(m).exists()`).
3. **`opts` (optional 3rd argument):**
   - If `nil`, defaults to `{}`.
   - Otherwise `type(opts)` must be `"table"`.
   - Every key in `pairs(opts)` must be a known option (`"prune"`). Any unknown key raises an error naming the key.
   - `opts.prune`:
     - When `nil`, defaults to `false`.
     - When non-nil, `type(opts.prune)` **must** be `"boolean"` (rejects `1`, `"yes"`, `{}`, etc.).

### Native Implementation (`core/datamile-lua/src/functions/fs/mod.rs`)

Update `_roots_native` to take `(String, Vec<String>, bool)`:

```rust
fn fs_roots(
    lua: &Lua,
    (dir, markers, prune): (String, Vec<String>, bool),
) -> mlua::Result<Table>
```

Traversal semantics:

1. Expand `dir` with `datamile_fs::expand_path(&dir)`.
2. Iterate `let mut it = WalkDir::new(&root).follow_links(true).into_iter();` with `while let Some(entry_res) = it.next()`:
   - Skip `Err(_)` entries.
   - Skip entries where `!entry.file_type().is_dir()`.
   - Check `markers.iter().any(|marker| entry.path().join(marker).exists())`.
   - On match:
     - Push `entry.path().canonicalize().unwrap_or_else(|_| entry.path().to_path_buf())` to `found`.
     - If `prune`, call `it.skip_current_dir()` so `WalkDir` does not descend into child directories of `entry`. (Because `!entry.file_type().is_dir()` was already filtered out, `entry` is guaranteed to be a directory, so `skip_current_dir()` skips `entry`'s children rather than its parent.)
3. Sort `found` naturally with `alphanumeric_sort::compare_path` and call `found.dedup()`.
4. Return a 1-indexed Lua array table of path strings.

### Interaction with Media Discovery & Hot Reload

- **`discover_roots` (`core/datamile-media/src/schema.rs`):**
  - When `prune = false` (default), behavior is unchanged and `reject_nesting` continues to detect accidental nested entity directories of the same `media_type`.
  - When `prune = true`, subdirectories of matched entity directories are never entered, trading nested-marker detection inside matched entities for faster discovery.
- **`watcher.rs`:**
  - `handle_known_root` calls `select_roots(engine, media_type, known)`. With `prune = true`, `fs_roots` matches `known` at depth 0, calls `it.skip_current_dir()`, and returns immediately without traversing `known`'s subdirectories.

## Call-Site Updates Required

No backwards compatibility is kept for `{ markers = ... }`:

1. `core/datamile-lua/src/functions/fs/tests.rs` (`test_d_fs_roots_finds_marker_directories`) — switch `{ markers = { 'album.toml', 'virtual.toml' } }` to `{ 'album.toml', 'virtual.toml' }`.
2. `docs/lua-api.md` — update `d.fs.roots` reference and example to `d.fs.roots(dir, marker, opts?)`.
3. `~/.config/datamile/album.lua` — update `d.fs.roots(dir, { markers = { "album.toml", "virtual.toml" } })` to `d.fs.roots(dir, { "album.toml", "virtual.toml" }, { prune = true })` (if opting into pruning).

## Test Plan

### Unit Tests (`core/datamile-lua/src/functions/fs/tests.rs`)

1. **String and array markers:** `d.fs.roots(root, 'album.toml')` and `d.fs.roots(root, { 'album.toml', 'virtual.toml' })` find matching directories, naturally sorted and canonicalized.
2. **Default `prune = false` visits nested matches:** With `A/album.toml` and `A/Disc 1/album.toml`, both `A` and `A/Disc 1` are returned.
3. **`prune = true` stops at matched directory:** On the same tree, `d.fs.roots(root, 'album.toml', { prune = true })` returns only `A` (and unrelated sibling `B`), omitting `A/Disc 1`.
4. **Pruning does not skip sibling or symlinked sibling directories:** Pruning `A` still visits `B` and symlinked `C`.
5. **Validation errors:**
   - `dir`: non-string (`123`, `nil`) or `""`.
   - `marker`: `""`, `"."`, `".."`, `{ "" }`, `{ "." }`, `{}`, `{ markers = { "album.toml" } }`, `{ [1] = "album.toml", prune = true }`, `{ [1] = "a", [3] = "b" }`, `123`.
   - `opts`: non-table (`true`, `"prune"`), unknown key (`{ prume = true }`), non-boolean `prune` (`{ prune = 1 }`, `{ prune = "yes" }`).

### Integration Tests (`core/datamile-media/src/tests.rs`)

1. Existing `discovery_rejects_nested_roots_within_one_media_type` continues to pass with default `prune = false`.
2. Add test confirming a config using `d.fs.roots(dir, "album.toml", { prune = true })` ignores nested `album.toml` inside an already-matched entity dir and succeeds without error.
