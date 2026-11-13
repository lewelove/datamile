local util = require("util")

local album = d.media.type("album")

album.lock(function(target)
    return d.fs.joinpath(target.path, "album.lock.json")
end)

-- Tag anchoring: a directory is an album root only when every audio file
-- shares one albumartist and album. Pruning keeps nested albums out of parents.
album.select(function(scope)
    return d.fs.roots(scope, function(_, entries)
        local audio = {}
        for _, entry in ipairs(entries) do
            if entry.type == "file"
                and d.fs.is_audio(entry.name, util.audio_extensions())
            then
                table.insert(audio, entry.path)
            end
        end

        if #audio == 0 then
            return false
        end

        local anchor
        local same = d.list.all(audio, function(path)
            local tags = d.get(d.fs.read_audio(path), "tags") or {}
            local artist = tags.albumartist or tags.artist
            if not artist or not tags.album then
                return false
            end

            local key = artist .. "\0" .. tags.album
            anchor = anchor or key
            return key == anchor
        end)

        return same, "prune"
    end)
end)

album.id(function(ctx)
    local music = d.get(REGISTRY.config, "storage.music_directory")
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

local function first_track(ctx)
    return (ctx.tracks or {})[1]
end

album.tag("albumartist", function(ctx)
    local track = first_track(ctx)
    return d.get(track, "tags.albumartist") or d.get(track, "tags.artist")
end)

album.tag("album", function(ctx)
    return d.get(first_track(ctx), "tags.album")
end)

album.tag("date", function(ctx)
    return d.get(first_track(ctx), "tags.date")
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
