A root media:

- Has a global discovery directory, in which initial scan will happen
- Have a stable `id` which cannot collide with other root media entities of the same name
- Must be a directory
- Have a way to discover and specify the table of directories, e.g. `select` funtion that must return a table of directories
- Produce the `<name>.lock.json` file for each of returned directories

A branch media:

- Must have a parent

A leaf media:

- Does not have an `id`, only `tags`, `objects`, `images`

Both branch and leaf medias must haev a deterministic way to order them. Add `index` function that will sort all medias by the value returned using natural order. w

```json
{
  "root_media_name": {
    "id": "",
    "tags": {},
    "objects": {},
    "images": {},
    "leaves": {
      "leaf_media_name": [
        {
        }
      ],
    },
  }
}
```
