local util = require("util")

local album = d.media("album", {
  kind = "root",
  dir = "~/Music",
  select = function(dir)
    return d.fs.roots(dir, "album.toml")
  end,
  children = { tracks = "track" },
})

album.input("manifest", function(ctx)
  return d.fs.read_toml(d.fs.joinpath(ctx.path, "album.toml"))
end)

album.file("manifest", function(ctx)
  return d.fs.joinpath(ctx.path, "album.toml")
end)

album.tag("albumartist", function(ctx)
  return d.get(ctx, "manifest.album.albumartist")
    or d.get(ctx, "manifest.album.artist")
end)

album.tag("album", function(ctx)
  return d.get(ctx, "manifest.album.album")
end)

album.tag("date", function(ctx)
  return d.get(ctx, "manifest.album.date")
    or d.get(ctx, "manifest.album.release_date")
end)

album.object("info", function(ctx)
  return util.info(ctx.tracks)
end)

album.image("main", function(ctx)
  local cover = util.cover(ctx.path)
  if not cover then
    return nil
  end

  return {
    fit = { "crop", 1080, 1080 },
    filter = "catmullrom",
    sources = cover,
  }
end)
