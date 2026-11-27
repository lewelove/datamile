# Spec: Strict Opt-In Pruning for `d.fs.roots` (`prune = true`)

## Status

Draft

## Context & Problem

`d.fs.roots` (`core/datamile-lua/src/functions/fs/mod.rs`, `core/datamile-lua/src/functions/fs/mod.lua`) currently walks the entire directory tree below `dir` without stopping when an entity directory matches a marker.

For an album layout such as `Artist - Album/{album.toml, Info/, Lyrics/, Digital Covers/, Disc 1/, Disc 2/}`, `fs_roots` continues descending into every subdirectory of every matched entity and checks all markers against each subdirectory. This affects:

1. Full discovery in `discover_roots` (`core/datamile-media/src/schema.rs`).
2. Update passes in `run_pass` (`core/datamile-media/src/update.rs`).
3. Hot-reload checks in `watcher.rs` (`handle_known_root` and `collect_media_changes`), where verifying a known entity dir still walks all of its subdirectories.

At the same time, `discover_roots` calls `reject_nesting` (`core/datamile-media/src/schema.rs`) to catch accidental nested entity directories within the same `media_type`. Stopping traversal at a match hides any nested marker below that directory, so pruning must be **strictly opt-in** rather than silent default behavior.

## Goals

- Add an explicit `prune` option to `d.fs.roots` that stops descending into subdirectories of any directory that matches a marker.
- Default `prune` to `false` so existing behavior and `reject_nesting` validation remain intact unless the caller explicitly opts in.
- Reject invalid or non-boolean `prune` values loudly at call time.

## Non-Goals

- Changing `d.fs.roots` default traversal behavior when `prune` is omitted.
- Replacing `WalkDir` or parallelizing `fs_roots` in this change (tracked separately in the bug reports below).
- Changing `d.fs.dir`, `d.fs.files`, or `d.fs.dirs`.

## API Design

### Lua Signature (`core/datamile-lua/src/functions/fs/mod.lua`)

```lua
d.fs.roots(dir, marker, opts?)
```

Accepted forms:

1. `d.fs.roots(dir, "album.toml")` -> `prune = false`
2. `d.fs.roots(dir, { "album.toml", "virtual.toml" })` -> `prune = false`
3. `d.fs.roots(dir, "album.toml", { prune = true })` -> `prune = true`
4. `d.fs.roots(dir, { "album.toml", "virtual.toml" }, { prune = true })` -> `prune = true`
5. `d.fs.roots(dir, { markers = { "album.toml", "virtual.toml" }, prune = true })` -> `prune = true`

### Strict Opt-In & Validation Rules

1. `dir` must be a `string` (existing check).
2. `marker` must be a `string` or `table`:
   - If `marker` is a `table` with `markers ~= nil`, `marker.markers` is used as the marker list, and `marker.prune` (if non-nil) supplies the prune flag.
   - Otherwise `marker` is treated as the marker list array.
3. `opts`, when non-nil, must be a `table`. Passing a non-table raises a Lua error.
4. Unknown option keys in `opts` (anything other than `prune`) raise a Lua error so typos fail fast instead of silently falling back to `prune = false`.
5. If `marker` is `{ markers = ..., prune = ... }` and `opts` is also provided with `prune ~= nil`, raise a Lua error to avoid ambiguous precedence.
6. `prune` defaults strictly to `false` when `nil`.
7. When `prune` is non-nil, `type(prune)` **must** be `"boolean"`. Truthy/falsy non-booleans (e.g., `prune = 1`, `prune = "true"`, `prune = {}`) must raise an error.
8. `markers` elements must be non-empty strings (or an empty table produces `{}` without error, matching current behavior).

### Native Binding (`core/datamile-lua/src/functions/fs/mod.rs`)

Update `_roots_native` signature from `(String, Vec<String>)` to `(String, Vec<String>, bool)`:

```rust
fn fs_roots(
    lua: &Lua,
    (dir, markers, prune): (String, Vec<String>, bool),
) -> mlua::Result<Table>
```

Traversal behavior in `fs_roots`:

- Iterate `WalkDir::new(&root).follow_links(true).into_iter()` with an explicit `while let Some(entry_res) = it.next()` loop.
- Skip `Err(_)` entries and any entry where `!entry.file_type().is_dir()`.
- Check `markers.iter().any(|marker| entry.path().join(marker).exists())`.
- If matched:
  - Record canonicalized path in `found`.
  - If `prune` is `true`, call `it.skip_current_dir()` so `WalkDir` does not descend into that directory's children. (Because `entry.file_type().is_dir()` was checked, `skip_current_dir()` always skips the matched directory's own subtree, never its parent.)
- Sort `found` naturally (`alphanumeric_sort::compare_path`) and `dedup()`.

### Interaction with `reject_nesting` & Watcher

- **`discover_roots` (`core/datamile-media/src/schema.rs`):** When a config opts into `prune = true`, nested directories inside a matched entity dir are never visited. `reject_nesting` still enforces no-nesting across any roots returned (e.g., if `select` combines multiple `d.fs.roots` calls or custom logic), but nested markers inside an already-matched root are intentionally not scanned.
- **`watcher.rs`:** `handle_known_root` (`select_roots(engine, media_type, known)`) passes `known` directly as `dir`. With `prune = true`, `fs_roots` matches `known` at depth 0, calls `it.skip_current_dir()`, and returns immediately without traversing `known`'s subdirectories.

## Test Plan

Add tests in `core/datamile-lua/src/functions/fs/tests.rs` and `core/datamile-media/src/tests.rs`:

1. **Default preserves nested detection (`prune` omitted / `false`):** A tree with `A/album.toml` and `A/Disc 1/album.toml` returns both `A` and `A/Disc 1`.
2. **`prune = true` stops at matched directory (both call forms):**
   - `d.fs.roots(root, "album.toml", { prune = true })`
   - `d.fs.roots(root, { markers = { "album.toml", "virtual.toml" }, prune = true })`
   Returns only `A` (and unrelated sibling `B/album.toml`), omitting `A/Disc 1`.
3. **Symlinked sibling directories:** Pruning `A` does not skip sibling directory `B` or symlinked sibling `C`.
4. **Strict validation failures:**
   - `prune = 1` or `prune = "yes"` errors.
   - `opts = "invalid"` errors.
   - Unknown option key `{ prume = true }` errors.
5. **Media engine integration (`core/datamile-media/src/tests.rs`):**
   - `discover_roots` with `prune = false` still triggers `reject_nesting` on nested `album.toml`.
   - `discover_roots` with `prune = true` succeeds without descending into matched entity subdirectories.

