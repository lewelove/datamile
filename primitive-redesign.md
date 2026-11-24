# Redesign of Primitives and Their Generators

This document outlines the mental model of primitives we use to interface our album library.

## Primitives

Primitives are the building blocks and the generator targets. We have two of them: Set and Sequence. Primitives are stored in memory at all times and can be called by their ids in combinatorial fashion to generate a view. Each view request can have an arbitrary amount of `sets` requested and a single `squence`. It works by taking a `sequence` list and then applying `set` bitmaps over it dynamically on each call.

### Set

A `Set` is a subset of a set of all albums. The `Set` data is:

```rust
pub struct Set {
    pub address: u64,
    pub bitmap: RoaringBitmap,
}
```

- `address` : BLAKE3 hash address of `bitmap`.
- `bitmap` : Bitmap containing all album uids matching a predicate.

It has related `match` predicate Lua function that runs through all albums and tells us: does an album belong in this set or not. Example:

```lua
match = function(a, value)
  return a.tags.genre == "Ambient"
end
```

The `value` is optional. It is used to inject data from outside of a `album.lock.json` at the evaluation time, like when generating groups.

### Sequence

A `Sequence` is a set of all albums, ordered in a specific way. The `Sequence` data is:

```rust
pub struct Sequence {
    pub address: u64,
    pub uids: Vec<u32>,
}
```

- `address` : BLAKE3 hash address of `uids`.
- `uids` : A vector containing all album uids in a specific order.

It has related `order` Lua function that returns keys by which albums will be ordered. Examples:

```lua
order = function(a)
  return a.tags.genre
end
```

```lua
order = function(a)
  return { a.tags.genre, a.tags.year }
end
```

It is always breaks tie by the `album.id`.

### Facet

A facet is a subset of a set of all albums ordered in a specific way + some arbitrary data. It has a single `set` and a single `sequence`. The `Group` data is:

```rust
pub struct Group {
    pub address: u64,
    pub set_address: u64,
    pub sequence_address: u64,
    pub data: ,
}
```

- `address` : BLAKE3 hash address of `set_address` + `sequence_address` + `data`.
- `set_address` : BLAKE3 hash address of a related `Set`.
- `sequence_address` : BLAKE3 hash address of a related `Sequence`.
- `data` : arbitrary data.

It has related `format` Lua function that takes in a group and returns any Lua object, which is then formatted to JSON one. The purpose of a `format` function is to populate specific cross section of `Set` and `Sequence` with metadata for UI to consume.

## Generators

Generators are the higher level abstractions that emit primitives and are run at library logic evaluation time
