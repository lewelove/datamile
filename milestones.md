# Dale Milestones

This document describes the system design and architecture milestones I have in mind for Dale.

## Lua Driven Manifest Engine

The `dale manifest` should be a Lua function. It is designed to manifest data from physical filesystem state into plaintext files. The definition is:

```lua
d.manifest("name", {
  cluster = function(dir) return { dir } end -- example of default implementation
  write = function(dir) 
    return {
      path = "manifest_name.toml/json",
      content = "" or {} -- either string to validate and write verbatim or table to be serialized
    }
  end
})
```

The `cluster` runs first. It takes the directory path from `--dir` option in CLI call. It must return an array of directory paths. All paths are canonicalized by Rust and validated to be directories and exist.

Then for every directory path in this array runs `write`. It always returns Lua table containing two keys: `file` and `content`.

The file format to write will be inferred from the `file` path extension. We allow both TOML and JSON as valid extensions.

If JSON and returned Lua table -> serialize into a sorted standard JSON -> write to path
If JSON and returned a string -> check the string to be valid JSON -> write to path unchanged -> else throw error

Same with TOML

## `Tantivy` Based Full Text Search Engine

## New `tags` and `objects`

Introduce new concepts: `tags` and `objects`.

The `tags` field replaces `keys` and adheres to the standard Vorbis comment notation. The object returned by the `dale.compile.album/track.tag` must be either a scalar, or a scalar list. If false -> throw compilation error. We also nest all root KV scalar data inside `tags` to unify structure. We define their existence through default overrideable Lua config shipped in the binary.

The `objects` filed has no such restrictions. Any valid JSON object can be written as an `object`. We introduce new `dale.compile.album/track.object` function similar to the `tag` one.

We also introduce some new UI functionality based on existence of supported `tags`, like `musicbrainz_albumid`, etc.
