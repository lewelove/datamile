# Datamile

A data-driven media library engine that compiles plain-text metadata beside immutable media files into compiled locks, then indexes, queries, and acts on them through Lua configuration.

## Media Schema

**Media root**:
The top-level declaration of a **Media type**, defining its **Init** directory, discovery rule, identity rule, and **Node** tree.
_Avoid_: root, media manifest

**Media type**:
The canonical name of a **Media root** (e.g., `album`, `book`, `movie`).

**Init**:
The filesystem directory a **Media root** scans to discover **Entities**.
_Avoid_: init.lua, root, library

**Entity**:
One compiled instance of a **Media root**, identified by its **Entity key**, owning one **Lock** and one id within its **Media type**.

**Entity dir**:
The canonical directory containing an **Entity**'s **Lock** and local **Entity manifest** files.
_Avoid_: root, album folder

**Entity key**:
The unique identity of an **Entity**: its **Media type** paired with its **Entity dir**.

**Node**:
A nested item inside an **Entity** (e.g., `track`, `season`, `episode`), optionally backed by a **Location**.
_Avoid_: branch media, leaf media

**Location**:
Where an **Entity** or **Node** lives on disk; absent for virtual **Nodes**.

**Tag**:
A scalar or list field on an **Entity** or **Node**, produced at **Compile** time.

**Object**:
A structured field on an **Entity** or **Node**, computed lazily at **Compile** time.

**Image**:
A named artwork namespace on an **Entity** or **Node**, rendered from source images at **Compile** time.

**Playable URI**:
The player-facing path of a file-backed **Node**, resolved from its **Location** relative to the player's media directory.

## Compilation & State

**Entity manifest**:
A plain-text source file inside an **Entity dir** that **Compile** hooks read (e.g., `album.toml`).
_Avoid_: manifest (unqualified), lock

**Lock**:
The compiled JSON artifact produced for one **Entity** inside its **Entity dir**. Always an output, never a **Compile** input.

**Compile**:
Evaluating a **Media root**'s hooks over one **Entity dir** to produce its **Lock**.

**Discovery**:
Running each **Media root**'s selection rule over its **Init** directory to find **Entity dirs**.

**Update pass**:
A full **Discovery** followed by incremental **Compile** over **Dirty** **Entities**.
_Avoid_: pass (unqualified), sync

**State entry**:
Per-**Entity** cached file fingerprints used to detect whether an **Entity** is **Dirty** without recompiling it.

**Dirty**:
An **Entity** whose **State entry** no longer matches disk and must be recompiled during an **Update pass**.
_Avoid_: stale

**External index**:
The in-memory map from external input files (outside any **Entity dir**) to the **Entities** that read them.
_Avoid_: index (unqualified), dependency index

**Entity index**:
The **Daemon**'s in-memory catalog of discovered **Entities** and their compiled **Locks**, holding **Playable URIs** and image references.
_Avoid_: index (unqualified), database

**Hot reload**:
Recompiling affected **Entities** or reloading configuration automatically in response to filesystem events.

## Logic

**Logic manifest**:
The declared set of **Libraries**, **Filters**, **Groupers**, **Orders**, **Shelves**, and **Cabinets** in configuration.
_Avoid_: manifest (unqualified)

**Library**:
A named, media-scoped **View** configuration pairing an eligibility predicate with allowed **Filters**, **Groupers**, and **Orders**.
_Avoid_: collection, init

**Filter**:
A named predicate restricting which **Entities** appear in a **Library** **View**.

**Grouper**:
A named selector extracting bucket keys from each **Entity** for sidebar grouping.

**Order**:
A named sort-key selector defining how **Entities** are ordered in a **View** or **Shelf**.

**Shelf**:
A named, ordered subset of **Entities** belonging to one **Media type**.

**Cabinet**:
A named collection of **Shelves** and **Orders** grouped together for an **Interface**.

**View**:
The ordered list of **Entity** ids produced by intersecting a **Library**, optional **Filter**, and search query, then applying an **Order**.

**Shelf view**:
A **Shelf**'s ordered **Entity** ids, optionally re-sorted by an **Order**.

## Actions & Interfaces

**Daemon**:
The long-running server process hosting the **Entity index**, **Hot reload** watcher, and HTTP/WebSocket API.

**Action**:
A named user-defined Lua operation invoked against resolved **Entity** targets (or targetlessly) with a runtime **Action context** and trailing **Options**.

**Action context**:
The runtime table passed to an **Action**: `{ lock, media, options, isatty }`, holding the target's parsed **Lock**, its **Media type**, the **Options**, and the terminal flag.

**Action map**:
The resolved set of **Actions** broadcast to clients, keyed by canonical name. A **Scoped action** carries one label and optional description per declared **Media type**; a **Generic action** carries one flat pair.

**Generic action**:
An **Action** with no **Media type** scope. It applies to every media type and accepts a targetless run.
_Avoid_: global action, untyped action

**Scoped action**:
An **Action** declaring one or more **Media types**. It is offered only for those types and needs resolved **Entity** targets.
_Avoid_: typed action, bound action

**Interface**:
A named client launcher declared in configuration (such as the web app).

**Option**:
Trailing command arguments passed verbatim to an **Action** inside its **Action context**.

## Relationships

- A **Media root** defines one **Media type**, scans one **Init** directory, and discovers many **Entities**
- An **Entity** is keyed by its **Entity key**, lives in an **Entity dir**, and compiles to one **Lock**
- An **Entity** contains nested **Nodes**, **Tags**, **Objects**, and **Images**
- A **Logic manifest** defines **Libraries**, **Filters**, **Groupers**, **Orders**, **Shelves**, and **Cabinets** scoped by **Media type**
- An **Update pass** checks **State entries** to find **Dirty** **Entities**, runs **Compile**, and updates the **Entity index** and **External index**
- An **Action** receives an **Action context** resolved from the **Entity index**
- An **Action map** lists **Generic actions** and **Scoped actions** for clients

## Flagged ambiguities

- **Library**: overloaded between a logic **Library** (`d.library`) and the user's whole collection or **Init** directory. Use **Library** only for the logic unit; use collection or **Init** for the rest.
- **Root**: overloaded between **Media root**, **Init** directory, **Entity dir**, and the filesystem root. Always qualify it.
- **Manifest**: overloaded between **Entity manifest** (`album.toml`) and **Logic manifest**. Always qualify it.
- **Index**: overloaded between **Entity index** and **External index**. Always qualify it.
- **ctx**: overloaded in Lua across **Compile** hooks, **Action context**, and **Interface** launchers. Always qualify in prose as compile context, **Action context**, or interface context.
