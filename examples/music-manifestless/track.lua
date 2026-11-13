-- Track definitions are shared by both music starter configs. Load this from
-- your own album type after requiring your utility module.
local util = require("util")

local track = d.media.type("track")

track.select(function(scope)
    return util.tracks(scope.path)
end)

track.input("audio", function(ctx)
    if ctx.audio ~= nil then
        return ctx.audio
    end
    if ctx.path == nil then
        return nil
    end

    return d.fs.read_audio(ctx.path)
end)

track.id(function(ctx)
    local tags = ctx.tags
    local disc = tonumber(tags.discnumber or 1)
    local number = tostring(tags.tracknumber or "")
    if disc > 1 then
        number = tostring(tags.discnumber) .. "." .. number
    end

    return table.concat({
        tags.albumartist or tags.artist or "",
        tags.album or "",
        number,
        tags.title or "",
    }, " - ")
end)

track.location(function(ctx)
    if ctx.path == nil then
        return nil
    end

    local stat = d.fs.stat(ctx.path)
    if not stat then
        return nil
    end

    return {
        kind = "file",
        path = ctx.path,
        mtime = stat.mtime,
        byte_size = stat.byte_size,
    }
end)

for _, key in ipairs({
    "artist",
    "albumartist",
    "album",
    "title",
    "tracknumber",
    "discnumber",
    "date",
    "genre",
    "composer",
}) do
    track.tag(key, function(ctx)
        return d.get(ctx, "manifest." .. key) or d.get(ctx, "audio.tags." .. key)
    end)
end

track.object("info", function(ctx)
    local audio = ctx.audio
    if not audio then
        return nil
    end

    return {
        sample_rate = audio.sample_rate,
        bits_per_sample = audio.bit_depth or 0,
        bitrate_kbps = audio.audio_bitrate or 0,
        encoding = audio.format,
        channels = audio.channels or 0,
        duration_milliseconds = audio.duration_milliseconds or 0,
        duration_formatted = d.fmt.clock(audio.duration_milliseconds or 0),
    }
end)
