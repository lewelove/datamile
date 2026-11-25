# Datamile Philosophy

Datamile is a barebones media library engine, not an opinionated media player. It gives you a fast Rust runtime and a Lua API, and leaves everything about what your collection *is* to your configuration.

---

## 1. Engine, Not Application

Think of Neovim. Out of the box, Neovim does not assume your editor layout, your plugins, your keybindings, or your workflow. It gives you a fast core, a Lua runtime, and a clean standard library (`vim.*`) so you can build the exact setup you want in `init.lua`.

Datamile takes the same approach for personal media collections:

- **Zero built-in media schemas.** Rust knows nothing about "albums", "tracks", "movies", "shows", "MusicBrainz", or "Discogs". If your `init.lua` is empty, Datamile manages nothing.
- **No defaults to fight.** You never have to override hidden behaviors or work around assumptions made by the engine. Your configuration *is* the entire definition of your collection.
- **Complete ownership from line one.** Whether you collect FLAC albums with TOML manifests, manifestless folders, multi-season TV shows, or chaptered audiobooks, you express the rules directly in Lua and own them completely.

---

## 2. No Hidden Magic

Magic makes simple demos look short and real configs impossible to reason about. Datamile avoids it everywhere:

- **Everything is visible in your config.** Nothing gets silently attached to your context behind your back. Every field your metadata functions read comes from what your own `ctx(...)` function returned or explicit `d.fs.*` calls.
- **Clear tree structure.** You declare top-level units with `d.media.root` and child levels with `d.media.node`. Every node has one parent and one root. Child lists are stored literally under `nodes.<name>`.
- **Independent outputs.** `.tag`, `.object`, and `.image` functions only read `(ctx, nodes)`. They never read each other, so line order never creates subtle bugs.
- **Fast-fail validation.** If a tree has a cycle, a missing node, or a node shared across two parents or two roots, the compiler exits immediately with an error before scanning a single folder. Silent guesses corrupt libraries; loud failures keep them clean.

---

## 3. Plain-Text Source, Compiled Truth

Datamile treats your collection like software source code:

- **Immutable media, mutable metadata.** Your audio, video, and book files stay untouched. Everything you curate—manifests, notes, lyrics, raw API scrapes, themes, metrics—lives beside them as plain-text files (`.toml`, `.json`, `.lrc`, `.txt`) that you can edit in any text editor and version with Git.
- **Compiled locks instead of opaque databases.** Each root directory compiles ahead-of-time into a readable `<name>.lock.json` file. There is no hidden SQLite database holding your collection hostage. You can inspect, search, diff, or pipe any entity through `jq` or any script.
- **Incremental by default.** Every read through `d.fs.*` records its file or directory dependency automatically. When you edit one file or drop one new folder, only that root recompiles—both in CLI updates and in the live `inotify` daemon.

---

## 4. A Small, Composable Standard Library (`d.*`)

Instead of baking dozens of rigid integrations into the binary, Datamile ships a compact Lua toolkit (`d.fs`, `d.tbl`, `d.list`, `d.str`, `d.json`, `d.toml`, `d.get`, `d.fmt`, `d.cache`, `d.system`, `d.dispatch`).

- **Familiar conventions.** The utility modules mirror Neovim's `vim.*` APIs so Lua users feel at home right away.
- **Bring your own tools.** External scrapers, taggers, renamers, and image analyzers stay outside the core as small CLI tools driven through `d.system` and `d.action`. If a tool breaks or you want a different workflow, you change a script or a few lines of Lua—not the engine.

---

## 5. Strict Separation of Concerns: Compile → Logic → Serve

Datamile splits the system into three clean layers that do not leak into each other:

1. **Compile (`d.media.root`, `d.media.node`):** Reads raw files on disk and produces verified, deterministic `<name>.lock.json` trees.
2. **Logic (`d.library`, `d.filter`, `d.grouper`, `d.order`, `d.shelf`, `d.cabinet`):** Pure Lua functions evaluated over compiled locks to slice, group, and sort your collection without knowing anything about how files are stored on disk.
3. **Serve & Interface (`d.interface`, `d.action`, HTTP/WebSocket API):** A decoupled backend that exposes compiled locks, logic views, cached images, playback, and user actions. The bundled web app is just one client—a TUI, script, or custom frontend talks to the exact same API.

---

## 6. Trade-Offs

This model is intentional about what it gives up:

- It is **not** a zero-config "point at a folder and press play" player. You have to write a `init.lua` describing your roots, nodes, and tags.
- In exchange, you get a library engine with zero lock-in, predictable performance at thousands of items, transparent data you can keep in Git, and total control over how your collection is structured and presented.
