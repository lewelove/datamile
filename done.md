## Redesign `/api/covers/`

`/api/covers/` is a simple GET HTTP request and must comply with web standards. The signature:

`/api/covers/{address}.{format}?size={positive_integer}&filter={filter}&quality={1-100}`

Let's define M as the `max(width, height)` of the master cover requested by address.

- `format` defines the image format send over the wire: `qoi`, `bmp`, `png`, `jpg`, `webp` (lossy). Required.
- `size` defines the resize parameter of the largest side of the image served. Falls back to the M.
- `filter` defines the interpolation algorithm to use for the resize. Defaults to `catmullrom`. Ignored if size == M.
- `quality` defines the quality of the lossy image served. For lossless formats it is ignored. For lossy defaults to 100.

## `/api/internal/prewarm_covers_cache`

This asynchronous POST endpoint is designed to be used by interfaces. It populates Dale's serverside cover cache for instant `/api/covers/` request. Interface decides which covers it will need beforehand, and POSTs a JSON containing the same parameters as in `/api/covers/`.

This endpoint is designed to fail fast for each requested cover candidate with already warm cache.

The parametrization combination logic must follow the `/api/covers/` one. `format`, `size`, and `filter` are required. The `quality` is required if and only if `format` chosen was lossy, defaults to 100 when omitted.

To filter covers scheduled for the prewarm we use `names` and `addresses` arrays.

- `names` : Filters all `album.covers` candidates.
- `addresses` : Filters all `album.covers.*.[].address` candidates.

At least one of them is required. The pattern:
- Only `names` / Only `addresses` -> run for specific filter.
- Both `names` & `addresses` -> combine filters.

Resulting files are saved under `{cache_dir}/covers/{address}/s{size}_{filter}[_q{quality}].{format}`, and are used as cache hit candidates for the `/api/covers/`. The `[_q{quality}]` is written ONLY for the lossy formats.

## New `compile.album.cover` Lua Function

The new function will allow to have multiple image files in album root as targets for pulling into a lock file. It returns either a single table with a `path` required or a table of such tables. 

It is used to specify the `master` compile target, the params are:

- `path` defines the relative to album root image path. Is either a string, or a table of strings, that define the fallback priority chain. If none of targets are valid image files, throw compilation error.
- `width` / `height` define the dimensions of bounding box in pixels.
- `fit` defines the fit algorithm. If `crop` -> fit in box by center crop. If `inside` -> fit in box without crop.
- `filter` defines the resize filter used.

All parameters are *required* and must be valid. Else throw compilation error.

Examples:

```lua
-- This one is default and is built in
dale.compile.album.cover("main", function(ctx, m)
  return {
    path = {
      "cover.png",
      "cover.jpg",
      "cover.jpeg",
      "folder.png",
      "folder.jpg",
      "folder.jpeg",
    }
    width = 1080,
    height = 1080,
    fit = "crop",
    filter = "catmullrom",
  }
end)
```

Each `name` populates the `covers` object in `album.lock.json`:

```json
{
  "covers": {
    "name": [
      {
        "address": "address_hash_string",
        "width": 1,
        "height": 1,
        "fit": "fit",
        "filter": "filter",
        "source": {
          "file": {
            "path": "path/to/image",
            "hash": "blake3-...",
            "mtime": 1,
            "byte_size": 1,
          },
          "info": {
            "width": 1,
            "height": 1,
            // etc bitmap image file metadata
          }
        }
      }
    ]
  }
}
```

The `address` is a first 16 chars of a `BLAKE3(source.file.hash, width, height, fit, filter) -> URL Safe Base64`.

The master cover is saved under `{cache_dir}/covers/{address}/master.qoi`

## Two Tier Cover Cache GC

Tier 1: Addressed Based Sweep. On each update of albums sweep delete all `{cache_dir}/covers/{address}/` directories that no longer belong to any album.

Tier 2: Derivative Time-To-Live. For each `/api/covers/` image served from cache (except `master.qoi`) update its `mtime` to `now`. On each `{cache_dir}/covers/` walk check each derivative image mtime against `cover_cache_ttl_days`. Prune if TTL is over.

