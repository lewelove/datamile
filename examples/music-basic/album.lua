local util = require("util")

local album = d.media.type("album")
local track = d.media.type("track")

album.lock(function(target)
    return d.fs.joinpath(target.path, "album.lock.json")
end)

album.select(function(scope)
    return d.fs.roots(scope, "album.toml")
end)

album.input("manifest", function(ctx)
    return d.fs.read_toml(d.fs.joinpath(ctx.path, "album.toml"))
end)

album.id(function(ctx)
    local music = d.get(REGISTRY.config, "storage.media_directory")
    if not music or music == "" then
        return nil
    end

    return d.fs.relpath(music, ctx.path)
end)

album.location(function(ctx)
    local stat = d.fs.stat(ctx.path)
    if not stat then
        return nil
    end

    return { kind = "directory", path = ctx.path, mtime = stat.mtime }
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

album.child("tracks", "track", function(ctx)
    return util.tracks(ctx.path)
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


d.action("open_manifest", {
    label = "Open Manifest",
    description = "Open album.toml in the default viewer",
    run = function(ctx)
        for _, entry in ipairs(ctx.albums) do
            local path = d.fs.joinpath(entry.path, "album.toml")
            if d.fs.exists(path) then
                d.system.open(path)
            end
        end
    end,
})
