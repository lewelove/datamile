# Implementation Runbook: `multi-locks` & `actions`

This document governs execution of `specs/multi-locks/spec.md` and
`specs/actions/spec.md`.

## Mandatory Workflow Rules

1. **One phase per session.** Implement **only** the phase requested (or the
   next unchecked phase below), then **stop**. Never start the next phase
   without explicit confirmation.
2. **Read before writing.** Before editing any code, read `AGENTS.md`,
   `CODING_STANDARDS.md`, `GLOSSARY.md`, `specs/multi-locks/spec.md`, and
   `specs/actions/spec.md`.
3. **TDD on behaviour changes.** For every changed or fixed behaviour, update
   or add the test first, observe it failing, implement the change, and observe
   it passing (`CODING_STANDARDS.md`).
4. **Stay inside the phase boundary.** Only edit files listed for the active
   phase (plus tests belonging to that phase). Do not add `#[allow]` without a
   `reason = "..."`, do not introduce `fn get_*`, and do not touch unrelated
   code.
5. **Green gates before committing.** Every phase must end with all three
   commands passing cleanly:
   - `devenv shell -- format`
   - `devenv shell -- lint`
   - `devenv shell -- tests`
6. **One phase = one commit.** Stage and commit the phase once all phase tests
   and workspace checks are green, using a clear commit message naming the
   phase. Leave the working tree clean before stopping.
7. **Never mutate live `~/.config/datamile/`.** `~/dotfiles` is a separate git
   repository. Any configuration migration (such as `actions.lua`) **must** be
   done in an isolated git worktree at `~/dotfiles/.worktrees/datamile/` on its
   own branch so the live `~/.config/datamile/` remains untouched. Verify
   worktree config changes with `datamile --config <worktree-datamile-dir>`.

---

## Phase Sequence

Execute strictly in order (`Step 1` -> `Step 7`). Check off a phase only after
its commit is created and green.

### Step 1 — `multi-locks` Phase 1: Identity, discovery, and lookups

- **Spec scope:** `specs/multi-locks/spec.md` (§1, §2 `remove_entity_by_type_and_path`, §5, §6, §7 MPD note).
- **Files:**
  - `core/datamile-cache/src/stat.rs`, `lib.rs` (`EntityKey`)
  - `core/datamile-logic/src/engine.rs`, `lib.rs`, `tests.rs` (`remove_entity_by_type_and_path`, `playable_lookup` `Vec`, `CachedImageLookup` / `ImageSpecMeta` with in-place `EntityKey` replacement)
  - `core/datamile-media/src/schema.rs`, `lib.rs`, `update.rs`, `watcher.rs`, `tests.rs` (per-type discovery/nesting, re-export `EntityKey`, update `remove_entity_by_path` callers)
  - `core/datamile-server/Cargo.toml`, `lib.rs`, `mpd_bridge.rs`, `api/images.rs`, `tests.rs` (add `datamile-cache` dep, re-export `EntityKey`, filter `"album"` in `mpd_bridge`, `CachedImageLookup.sources` fallback)
- **Verify:**
  - `cargo test -p datamile-logic -p datamile-media -p datamile-server`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 2 — `multi-locks` Phase 2: Freshness and `CompileGuard`

- **Spec scope:** `specs/multi-locks/spec.md` (§7).
- **Files:**
  - `core/datamile-cache/src/stat.rs`, `stat/tests.rs` (`walk_files` skips `*.lock.json`, `RootState.lock_stat`, `root_is_dirty(dir, media_type, walked, entry)`)
  - `core/datamile-lua/src/lib.rs`, `functions/fs/mod.rs`, `functions/fs/tests.rs`, `tests/memo.rs` (RAII `CompileGuard`, enforce `*.lock.json` read ban at `EngineContext::record_dependency`, filter `*.lock.json` in `fs` enumerators while compiling)
  - `core/datamile-media/src/compiler.rs`, `deps.rs`, `update.rs`, `tests.rs` (hold `CompileGuard` in `compile_root_impl`, `CompiledRoot.lock_stat`, `StateEntry = (PathBuf, RootFiles, FileStat)`, `plan_work` passing `media_type`, record `lock_stat` for all compiled roots, `PermissionDenied`/`EROFS` entity error isolation)
- **Verify:**
  - `cargo test -p datamile-cache -p datamile-lua -p datamile-media`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 3 — `multi-locks` Phase 3: Single write path

- **Spec scope:** `specs/multi-locks/spec.md` (§2 `UpdateScope`/`UpdateRequest`/`force_update_entity`, §3 `ExternalIndex`, §4 Watcher & single write path).
- **Files:**
  - `core/datamile-server/src/backend.rs` (`UpdateRequest.entities: Vec<EntityKey>`)
  - `core/datamile-media/src/state.rs`, `deps.rs`, `update.rs`, `watcher.rs`, `backend.rs`, `tests.rs` (`MediaState::init_dirs()` via `media_roots()`, `ExternalIndex = BTreeMap<PathBuf, BTreeSet<EntityKey>>`, `UpdateScope` + complete `merge` matrix, `UpdateScope::Paths`-only empty bail, `force_update_entity` sets `entities`, `watcher.rs` submits `EntityKey`/`Paths` to `run_server_update` without holding locks)
- **Verify:**
  - `cargo test -p datamile-media -p datamile-server`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 4 — `actions` Phase 1: Lua declarations, schema validation, and `dict.path`

- **Spec scope:** `specs/actions/spec.md` (§1, §2, §3, §4 `dict.path` + domain types, §7 `allowed_origins` parsing).
- **Note for green build:** Keep `ServerConfig.actions` compatible in this commit by projecting `ActionMap` -> `ActionDef` at `datamile-media/src/backend.rs` until Step 5 switches `datamile-server` to `ActionMap` and `ActionCtx`.
- **Files:**
  - `core/datamile-lua-domain/src/config.lua`, `core.lua`, `config.rs`, `engine.rs`, `lib.rs`, `tests.rs` (`d.action` canonicalization/duplicates, `__DATAMILE_GET_ACTIONS` -> `ActionDeclarations`, `__DATAMILE_RESOLVE_ACTIONS` via `resolve_actions`, `ActionPresentation`/`ActionEntry`/`ActionMap`/`ActionCtx`, `ServerConfig.allowed_origins` parsing/CIDR validation)
  - `core/datamile-media/src/schema.rs`, `ingest.rs`, `backend.rs`, `tests.rs` (`validate_media_schema` checks `&ActionDeclarations`, `ingest.rs` adds `"path": root` to `dict`, `backend.rs` calls `resolve_actions` after `validate_media_schema`)
- **Verify:**
  - `cargo test -p datamile-lua-domain -p datamile-media`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 5 — `actions` Phase 2 + `multi-locks` Phase 4 (resolver): Unified `POST /api/x`, `ActionMap` cutover, and LAN security

- **Spec scope:** `specs/actions/spec.md` (§4, §5, §7) and `specs/multi-locks/spec.md` (Target resolution).
- **Files:**
  - `core/datamile-media/src/state.rs`, `backend.rs`, `watcher.rs`, `tests.rs` (drop `MediaState.actions` and `ActionTargetQuery` impl, pass `host`/`allowed_origins`/`ActionMap` into `ServerConfig`, broadcast `ActionMap` in `LOGIC_UPDATE`)
  - `core/datamile-server/src/state.rs`, `backend.rs`, `lib.rs`, `api/mod.rs`, `api/websocket.rs`, `api/system.rs`, `tests.rs` (switch to `ActionMap`/`ActionCtx`, delete `ActionTargetQuery`, bind `config.host`, install `Host`/`Origin` middleware with IPv6 bracket port stripping + `CorsLayer`, add `Host` header to existing POST tests)
  - Create `core/datamile-server/src/api/x.rs`; delete `core/datamile-server/src/api/actions.rs` and `select.rs` (implement `POST /api/x` with `execute: bool` and the shared `LogicEngine` target resolver)
  - `core/datamile/src/main.rs` (temporary compilation update for removed `ActionTargetQuery` / `execute_action` signature ahead of Step 6)
- **Verify:**
  - `cargo test -p datamile-media -p datamile-server`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 6 — `actions` Phase 3: CLI `datamile x` and integration tests

- **Spec scope:** `specs/actions/spec.md` (§6).
- **Files:**
  - `core/datamile/src/main.rs`, `Cargo.toml` (narrow `XArgs` to `--dir`, `--media`, `-- <options>...`; require running daemon via `InstanceLock` + `/api/identity`; evaluate `init.lua` via `DomainLuaEngine` without `discover_roots`/`scan_library`; resolve `--dir` / cwd locally; call `POST /api/x` with `execute: false`; run `execute_action` locally or output `intermediary` JSON array)
  - Create `core/datamile/tests/x_cli.rs` (integration tests against a live server instance covering offline refusal, cross-product order, partial failures, targetless generic, and `intermediary`)
- **Verify:**
  - `cargo test -p datamile`
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit, then STOP.**

### Step 7 — `actions` Phase 4 + `multi-locks` Phase 4 (docs) + dotfiles worktree migration

- **Spec scope:** `specs/actions/spec.md` (§4 web app, Risks migration) and `docs/` / `README.md` alignment.
- **Files in `datamile` repo:**
  - Create `interfaces/web-app/src/library/actions.ts` and `src/library/actions.test.ts`
  - Modify `interfaces/web-app/src/types.ts`, `api.ts`, `library/collection.svelte.ts`, `modules/HomeView/ModalDrawer/ModalDrawer.svelte`, `modules/NavigationBar.svelte`, `App.svelte`
  - Modify `interfaces/web-app/package.json` and `devenv.nix` (add `bun test` to `scripts.tests`)
  - Verify `docs/lua-api.md` and `GLOSSARY.md` match the final implementation (including any newly introduced terms such as `Entity key`, `Generic action`, `Scoped action`, and `Action map`); update `README.md` action examples (`/api/x` and `ctx.lock[ctx.media]`)
- **Verify in `datamile` repo:**
  - `devenv shell -- format && devenv shell -- lint && devenv shell -- tests`
- **Commit in `datamile` repo.**
- **Dotfiles migration (isolated worktree only — do NOT edit `~/.config/datamile/`):**
  - Inspect `~/dotfiles` and create a worktree at `~/dotfiles/.worktrees/datamile/`:
    ```bash
    git -C ~/dotfiles worktree add ~/dotfiles/.worktrees/datamile -b datamile-multi-locks-actions
    ```
  - Migrate the datamile config files inside `~/dotfiles/.worktrees/datamile/` (e.g. `actions.lua`: `ctx.items` -> `ctx.lock[ctx.media]`, string `ctx.options` -> `d.list.contains(ctx.options, ...)`, `media` scopes; check `init.lua` / `album.lua` for any lock-read or `server.host` updates).
  - Verify the worktree config compiles and loads cleanly without touching live `~/.config/datamile`:
    ```bash
    datamile --config <path-to-worktree-datamile-config-dir> compile "<test-album-dir>"
    ```
  - Commit inside `~/dotfiles/.worktrees/datamile/` and **STOP**. Report the worktree branch and commit hash so the user can review and merge into `~/dotfiles` when ready.
