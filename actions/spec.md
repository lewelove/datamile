# Actions: media-scoped action contract

Status: proposed. Not approved. Records the feedback from the 2026-11-24
session.

## Intent

Datamile must stay media agnostic. Today the action registry is one flat
namespace and the web app hardcodes `type: "album"`. An action should
declare which media roots it belongs to, present a different label and
description per root, and receive the root it runs for.

## Terms

Every word here is defined in `docs/vocabulary.md`: media type, media root,
`init`, media entity, entity dir, id, entity manifest, media manifest, logic
manifest, lock, entity index, `REGISTRY`, getter, `location.path` and `ctx`.
The only shape this document fixes is `ctx.lock`, which is the targeted
entity's lock file, parsed (see *No `ctx.media`*).

## The model

- `media = nil` -> generic. Not tied to a root. `nil` means no media was
  written; it never falls back to `album`.
- `media = "album"` -> one root.
- `media = {"album", "movie"}` -> several roots.
The key of `actions.media`, every value of `--media`, the `media` argument of
`run` and the single key of a lock are the same media-type string.

- `label` / `description`: `nil`, a non-empty string, or
  `function(media)` returning a non-empty string. Nothing else.
- `run`: `function(ctx, media)`. Lua ignores extra arguments, so a generic
  action can write `function(ctx)`.
- WYSIWYG: whatever the declaration receives is what the resolver passes.
  `label(media)`, `description(media)`, `run(ctx, media)`.

```lua
d.action("open_terminal", {
  media = {"album", "movie"},
  label = function(media)
    return media == "album" and "Open Album Terminal" or "Open Movie Terminal"
  end,
  description = function(media)
    return "Open a terminal in the " .. media .. " directory"
  end,
  run = function(ctx, media)
    d.system({ "alacritty" }, { cwd = ctx.lock[media].location.path, detach = true })
  end,
})

d.action("open_config_in_terminal", {
  label = "Open Config in Terminal",
  run = function(ctx)
    d.system({ "alacritty" }, { cwd = "~/.config/datamile", detach = true })
  end,
})
```

## Decisions

### 1. One declaration per name; a second is an error

`d.action(name, ...)` stores and never merges. A second declaration of the
same name fails config load.

```lua
-- actions/album.lua
d.action("open_terminal", { media = "album", run = album_terminal })
-- actions/movie.lua
d.action("open_terminal", { media = "movie", run = movie_terminal })
-- error: action 'open_terminal' is declared twice
```

The error is loud because the fix is local: one declaration lists every
media the action operates on. Detection sits in `d.action` itself, which
rejects a name already present in `REGISTRY.actions`, so it fires at the
call and not at expansion.

Consequence: `run` is required on every declaration. The current
`run = t.run or existing.run` merge goes away, and `label` /
`description` default to `nil` instead of inheriting.

### 2. Resolution happens after `init.lua`, not in `d.action`

`d.action` only stores the declaration. `__DATAMILE_GET_ACTIONS` (already
called after evaluation) produces the action set from the declaration
itself:

- `media = nil` -> one entry in `generic`.
- `media = "album"` -> one entry under `media.album`.
- `media = {"album", "movie"}` -> one entry under each declared media.

Nothing walks the root list. An action appears under the media it declares
and nowhere else.

Why after `init.lua`: both the declarations and the media roots are final
only once it returns.

```lua
-- album.lua   d.media.root("album", ...)
-- movie.lua   d.media.root("movie", ...)
-- actions.lua d.action("open_config_in_terminal", { run = ... })  -- media = nil
```

Roots may be declared after the action, in another `require`, so
`{"album", "movie"}` can only be checked against them once `init.lua`
returns. This mirrors `__DATAMILE_GET_LOGIC_MANIFEST`, which is also
resolved after evaluation.

### 3. Validation

Shape, in the getter:

- `name` must be a non-empty string.
- `media` must be `nil`, a non-empty string, or a table of non-empty
  strings. An empty string or an empty table is an error.
- Dedup repeated entries in one media list.
- `run` is required and must be a function.
- `label` / `description` must be `nil`, a non-empty string, or a
  function returning a non-empty string. A function returning `nil` is an
  error: there is no "hide this action for this media".
- `label` nil -> fall back to the action name. `description` nil -> omit.
- A function is called once per media during resolution; for a generic
  action it is called once with `nil`. A bad return fails config load.

Media names are checked in Rust, where the media manifest is already in hand
and where filters, groupers, orders, libraries, shelves and cabinets are
checked. Actions join that loop and reuse its message instead
of growing a second validator in Lua:

```console
d.action("open_book", { media = "book", run = print })   -- no book root
error: action 'open_book' declares unknown media type 'book'
```

### 4. No `ctx.media`

`ctx` holds `lock`, `options` and `isatty` (true when stdin is a terminal).
`ctx.lock` is the parsed lock file of the targeted entity, and everything
about that entity is inside it. The media type is the second `run` argument,
`nil` for generic actions, and it is the key into the lock:
`ctx.lock[media]`. The engine always calls `run(ctx, media)`.

`ctx.lock` is `nil` when the run has no target. A generic action invoked
without an entity, such as `open_config_in_terminal`, gets `ctx.lock = nil`
and reads nothing from it.

`options` is an array of strings built from everything after `--`:

```console
$ datamile x open_terminal --dir /media/abbey-road -- --how --fast
  ctx.options = { "--how", "--fast" }
```

Rust splits the argv and hands Lua that list; nothing parses it further.
`--query` no longer exists, so `--` is the only source of options.

Because `run` carries the media, the Lua registry keeps one run per
action, not one per media. The per-media projection materializes only
`{label, description}`.

### 5. Client shape: generic bucket plus nested per media

```json
"actions": {
  "generic": {
    "open_config_in_terminal": { "label": "...", "description": "..." }
  },
  "media": {
    "album": {
      "open_terminal": { "label": "Open Album Terminal", "description": "..." }
    },
    "movie": {
      "open_terminal": { "label": "Open Movie Terminal", "description": "..." }
    }
  }
}
```

Client lookup, with `type` the media type of the entity being rendered:

```text
action(name, type):
  if type is not null and actions.media[type] has name: return it
  return actions.generic[name] or nothing
```

The `generic` branch can only fire for an action declared with `media = nil`,
because names are unique across declarations: a name found under
`media[type]` never also exists in `generic`.

The client change touches these files (plus adding `location` to the entity
dict in `ingest.rs` so the client always has `location.path` synchronously):

- `core/datamile-media/src/ingest.rs`: include `"location": entity.get("location")`
  in `dict` so `collection.dict[id].location.path` is available without waiting
  for `ensureFullAlbum`.
- `interfaces/web-app/src/api.ts`: `executeAction(name, media, dir, options?)`
  posts `{ name, dir, media, options }` to `/api/x`. Eight call sites pass
  `(media, dir)` (or `(null, null)` for targetless generic actions): four in
  `ModalDrawer.svelte`, three in `App.svelte`, one in `NavigationBar.svelte`.
- `interfaces/web-app/src/library/collection.svelte.ts`: `hasAction` becomes
  `action(name, type)` over the nested set, and the store keeps the media
  type of the rendered entity instead of `json.media?.album`.
- `interfaces/web-app/src/types.ts`: `INIT` and `LOGIC_UPDATE` carry the nested
  `ActionSet`, `AlbumDictEntry` includes `location`, and every
  `Record<string, ActionDef>` reader changes.
- `ModalDrawer.svelte` and `NavigationBar.svelte`: the eleven
  `collection.actions[name]?.label` reads become `action(name, type)?.label`.

The reason it is unavoidable: the flat map the client reads today is exactly
what this document replaces.

### 6. Rust keeps one structure, and splits index lookup from execution

```rust
struct ActionDef {
    label: String,
    description: Option<String>,
}

struct ActionSet {
    generic: BTreeMap<String, ActionDef>,
    media: BTreeMap<String, BTreeMap<String, ActionDef>>,
}
```

`ActionDef` carries presentation only. The function to call stays in the
Lua registry, one per action name. The structs live in
`datamile-lua-domain/src/config.rs`; `datamile-server` re-exports them,
`EvaluatedDomainData.actions` takes the same type, and the dead
`MediaState.actions` copy is deleted.

Two routes keep the daemon as the owner of the live entity index while letting
CLI actions keep the user's terminal (`isatty`, `/dev/tty`, `stdio = "inherit"`):

1. **`POST /api/x`** (HTTP / web-app clients — daemon executes the action):
   ```text
   POST /api/x        { "name": "open_terminal", "dir": "/media/abbey-road",
                        "media": "album", "options": [] }
     -> target: album/abbey-road                     from the entity index
     -> gate:   actions.media["album"]["open_terminal"] exists   allowed
     -> run:    REGISTRY.actions.open_terminal(ctx, "album")
        ctx = { lock = { album = { ... } }, options = {}, isatty = false }
   ```
   - For a **scoped** action, `dir` and `media` are both required.
   - For a **generic** action (`actions.generic`), `dir` and `media` may both
     be `null` / omitted (targetless invocation -> `ctx.lock = nil, media = nil`,
     e.g. `open_config_in_terminal` from the nav bar). Supplying one without the
     other is a `400 Bad Request`. When both are supplied, the entity is
     resolved and handed to `run(ctx, nil)`.

2. **`POST /api/x/resolve`** (CLI target resolution — CLI executes locally):
   - Takes `{ "name": String, "dirs": Vec<String>, "media": Vec<String>, "explicit_target": bool }`
     (or per-run query) and validates the action guard + resolves locks from the
     daemon's live entity index without running Lua.
   - `datamile x` requests the resolved `(lock, media)` run list from
     `/api/x/resolve` when the daemon holds `InstanceLock` (or builds it locally
     via `scan_library` when offline), then calls `backend.execute_action`
     **inside the CLI process** so `isatty = std::io::stdin().is_terminal()`,
     `/dev/tty` prompts (`embed`, `rename`), and `stdio = "inherit"` work
     identically whether the daemon is running or not.
   - `datamile x intermediary` prints the resolved `ctx` array (the exact
     payloads each run receives).

These replace `POST /api/actions/{name}` and `POST /api/select`. Both routes
share the same resolver and guard logic:

- A dir and media pair the index does not hold is an error naming
  `datamile update`.
- `ServerConfig.actions: ActionSet` in `datamile-server/src/state.rs`. The WS
  `INIT` / `LOGIC_UPDATE` serialize it as-is, with `BTreeMap` ordering, so the
  `HashMap`-to-`BTreeMap` conversion in `websocket.rs` is deleted.
- `run(ctx, media)` crosses these interfaces: `MediaBackend::execute_action`
  and its media implementation gain `media: Option<&str>`,
  `DomainLuaEngine::execute_action` passes it as the second `run_fn.call`
  argument, both Rust callers (`api/x.rs` and `main.rs`) build
  `ctx = { lock, options, isatty }` instead of `{ items, options, isatty }`,
  and the server test mock follows.

### 7. No `config` field

`d.action` stores `config = t.config or existing.config or {}` and nothing
reads it. `ActionDef` carries `label` and `description` only, so the field is
dropped in the same change. A declaration that still passes `config` is
ignored, like any other key `d.action` does not read.

## Structuring away the `generic` collision

The collision only exists when media types are direct siblings of the
generic bucket:

```json
"actions": { "generic": { ... }, "album": { ... } }
```

A media root named `generic` is legal (`[a-z0-9_-]`), so the two would
share a key. Two structural fixes:

1. **Two namespaces**: `actions.generic` for generic
   actions, `actions.media[type]` for scoped ones. A media root named
   `generic` is `actions.media.generic`; the generic bucket is
   `actions.generic`. Different levels, no overlap, no reserved name.
   Rust `ActionSet { generic, media }` serializes to exactly this.

2. **Array of groups**:
   ```json
   "actions": [
     { "media": null, "actions": { ... } },
     { "media": "album", "actions": { ... } }
   ]
   ```
   Collision-proof by construction, no reserved keys, but lookup scans
   the array and a future scope shape needs a schema.

Rejected: flat `actions[type]` with `generic` reserved. That is a rule
enforced by validation, not a structure.

## What per-type separation buys

The action set is per media type for **presentation, validation and the
execution gate**, not for dispatch:

- The client offers only the actions declared for the entity it renders.
  A movie must not show `open_terminal` when no movie action declares it.
- A scoped action invoked for a type it was not declared for is an error,
  not a silent no-op.
- `label` / `description` can differ per type.

The split does not reach dispatch: the lock's single top-level key is the
media type, so a run could read it from `ctx.lock`. The cross product of
`--dir` and `--media` keeps every run homogeneous anyway, so `media` is
definite for a scoped action.

Dropped alternative: no second `run` argument, the run reads the lock's key.
It saves one parameter. `run(ctx, media)` stays because `media` is the key
into the lock (`ctx.lock[media]`), and because a scoped action receives the
declared media the gate already checked.

A generic action receives `media = nil` and takes the type from the lock's
only key (if targeted), or receives `ctx.lock = nil` when targetless.

## Multi-lock dirs

A dir holds one entity per media type that selects it, so two locks in one dir
are two entities. That is recorded in `specs/multi-locks/spec.md`, which owns
discovery, target resolution, staleness and the lock lifecycle.

## Invocation

`x` is the subcommand that runs one action by name:

```console
$ datamile x open_terminal --dir /media/abbey-road --media album
```

It exists today with a wider set of target flags. This document narrows it to
`--dir` and `--media`: `--type` becomes `--media`, `--directory` becomes
`--dir`, the other target flags are dropped, and options come from after
`--`. Target selection, the cross product and the exit code are defined in
`specs/multi-locks/spec.md`, *Target resolution*. This document adds one rule:
the action's declared media is a guard.

- A pair whose media type the action does not declare is an error for that
  pair, and the remaining pairs still run.
- A generic action (`media = nil`) accepts any media type the dir resolves to
  and receives `media = nil`. When invoked with neither `--dir` nor `--media`,
  it runs once with `ctx.lock = nil` and `media = nil`.
- The targeted run receives the entity's lock as `ctx.lock`.

A dir holding `{album,cd,vinyl}.lock.json`:

```console
$ datamile x open_terminal --dir /media/abbey-road --media album
$ datamile x open_terminal --dir /media/abbey-road --media cd
error: action 'open_terminal' does not declare 'cd'
```

## Known consequences

- An action runs once per run, which is one `(dir, media)` pair (or once with
  `ctx.lock = nil` for a targetless generic action). An action that used to
  receive a list of entities receives one entity per run and is invoked again
  for the next one.
- The old selectors leave `x`. A caller that used `--query`, `--playing`,
  `--id`, `--recursive` or `--library` passes the entity dir with `--dir` and
  the media type with `--media` instead.
- `POST /api/actions/{name}` and `POST /api/select` are replaced by
  `POST /api/x` (daemon execution for HTTP clients) and `POST /api/x/resolve`
  (index lookup for local CLI execution).
- Existing user configs (such as `~/.config/datamile/actions.lua`) must be
  updated for `ctx.lock[media]` and `ctx.options` as a string array.

## Non-goals

- Changing the logic manifest (`filters`, `groupers`, `orders`, logic
  libraries, `shelves`, `cabinets`) to the per-media-type shape that `actions`
  uses.
- Making the web app render entities of any media type beyond passing the
  rendered entity's type.

## Reversals

Three decisions were reversed during review.

- A second `d.action` for one name is an error, not a shadow. Shadowing lost
  the earlier media silently, and the call site could not see the loss.
- `label` / `description` are a string or a function of `media`, not a table.
  A function covers per-media interpolation without a new declaration form.
- `--media` is never inferred from the dir. Inferring it needs a multi-branch
  resolution rule, and one explicit flag needs none.

## Implementation

### Files

- `core/datamile-lua-domain/src/config.lua`: `d.action` stores without
  merging, rejects a duplicate name, drops `config`.
- `core/datamile-lua-domain/src/core.lua`: `__DATAMILE_GET_ACTIONS` returns
  the nested set.
- `core/datamile-lua-domain/src/config.rs`: `ActionDef` and `ActionSet`.
- `core/datamile-lua-domain/src/engine.rs`: `ctx` becomes
  `{ lock, options, isatty }`, and `run` receives `media: Option<&str>`.
- `core/datamile-media/src/schema.rs`: `validate_media_schema` also checks
  action media names and reuses `require_media`.
- `core/datamile-media/src/ingest.rs`: include `location` in entity `dict`.
- `core/datamile-media/src/state.rs`: `MediaState.actions` is deleted.
- `core/datamile-server/src/state.rs`: `ServerConfig.actions: ActionSet`.
- `core/datamile-server/src/api/x.rs`: new, `/api/x` and `/api/x/resolve`.
- `core/datamile-server/src/api/actions.rs` and `select.rs`: deleted.
- `core/datamile-server/src/api/websocket.rs` and `system.rs`: send the set
  as-is, without the `HashMap` to `BTreeMap` conversion.
- `core/datamile/src/main.rs`: `XArgs` narrowed, `x` resolves via
  `/api/x/resolve` (or local scan) and runs locally; `intermediary` prints the
  resolved `ctx` array.
- `interfaces/web-app/src/api.ts`, `types.ts`,
  `library/collection.svelte.ts`,
  `modules/HomeView/ModalDrawer/ModalDrawer.svelte`,
  `modules/NavigationBar.svelte`, `App.svelte`.
- `docs/lua-api.md` sections 9, 10.1 and 11, and `README.md`.

### Phases

Each phase builds, and `devenv shell -- lint` passes at the end of it.

1. Shape: `ActionDef` and `ActionSet`, the getter, `d.action`, and `ctx`. The
   old wire is kept by converting at the edge, so the daemon and the client
   still talk.
2. Validation: actions join `validate_media_schema`.
3. Route: `POST /api/x` and `POST /api/x/resolve` replace `actions` and
   `select`, and `x` uses `/api/x/resolve` with local execution.
4. Client: the nested set, `action(name, type)`, `location` in `dict`, and the
   typed `executeAction`.
5. Docs.

Nothing ships before phase 4 lands. The wire and the client must change in the
same release, or the client renders no action at all.

### Tests

- `datamile-lua-domain`: duplicate name errors; `media = ""` and `media = {}`
  error; a label function returning nil errors; the function is called once
  per declared media with that media; a generic action produces one entry in
  `generic`; `ctx.lock = nil` for a run with no target.
- `datamile-media`: an unknown media type in a declaration is rejected by
  `validate_media_schema` with the shared message; a generic action accepts
  any resolved media type.
- `datamile-server`: `/api/x` resolves `dir` and `media`, allows targetless
  generic invocation (`dir = null, media = null`), rejects partial targets with
  400, gates a scoped pair, returns the CLI error text, rejects an unknown
  entity with a message naming `datamile update`, `/api/x/resolve` returns locks
  for CLI runs, and `INIT` and `LOGIC_UPDATE` both carry the nested set.
- `interfaces/web-app`: the lookup falls back to `generic`; `action(name, type)`
  hides an action declared for another media type.
- `datamile`: one failing pair still runs the rest and the command exits
  non-zero; generic action with no `--dir` or `--media` runs locally with
  `ctx.lock = nil` and `isatty` preserved.


