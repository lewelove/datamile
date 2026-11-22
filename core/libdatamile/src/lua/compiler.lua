_G.datamile = _G.datamile or {}
_G.d = _G.datamile
_G.datamile.compile = _G.datamile.compile or {}

local function is_scalar(v)
    local t = type(v)
    return t == "string" or t == "number" or t == "boolean"
end

local function describe(owner, name, index)
    if index then
        return string.format("%s tag '%s' at index %d", owner, name, index)
    end
    return string.format("%s tag '%s'", owner, name)
end

-- A tag is one scalar or a list of scalars. Returning anything else is a config
-- bug, so it fails the compile instead of landing in the lock half-formed.
local function check_tag(owner, name, value, index)
    if value == nil or value == "" then
        return nil
    end

    if type(value) ~= "table" then
        if not is_scalar(value) then
            error(
                string.format(
                    "Error evaluating %s: expected a scalar or a list of scalars, got %s",
                    describe(owner, name, index), type(value)
                ),
                0
            )
        end
        return value
    end

    if next(value) == nil then
        return nil
    end

    for k, item in pairs(value) do
        if type(k) ~= "number" or not is_scalar(item) then
            error(
                string.format(
                    "Error evaluating %s: expected a scalar or a list of scalars, got a table with non-scalar content",
                    describe(owner, name, index)
                ),
                0
            )
        end
    end

    return value
end

-- Objects take any JSON. Empty means absent, so nothing is written for it.
local function check_object(value)
    if value == nil or value == "" then
        return nil
    end
    if type(value) == "table" and next(value) == nil then
        return nil
    end
    return value
end

_G.datamile.compile.album = {
    tag = function(name, v)
        if type(v) == "function" then
            REGISTRY.tags.album[name] = v
        elseif v == true then
            REGISTRY.tags.album[name] = function(ctx, m)
                return m.metadata and m.metadata.album and m.metadata.album[name]
            end
        end
    end,
    object = function(name, v)
        if type(v) == "function" then
            REGISTRY.objects.album[name] = v
        end
    end,
    id = function(v)
        if type(v) == "function" then
            REGISTRY.id_fn = v
        end
    end,
    image = function(name, v)
        if type(v) == "function" then
            REGISTRY.images_rules[name] = v
        end
    end
}

_G.datamile.compile.tracks = {
    tag = function(name, v)
        if type(v) == "function" then
            REGISTRY.tags.tracks[name] = v
        elseif v == true then
            REGISTRY.tags.tracks[name] = function(ctx, m, i)
                return m.metadata and m.metadata.tracks and m.metadata.tracks[i] and m.metadata.tracks[i][name]
            end
        end
    end,
    object = function(name, v)
        if type(v) == "function" then
            REGISTRY.objects.tracks[name] = v
        end
    end
}

_G.datamile.compile.a = _G.datamile.compile.album
_G.datamile.compile.track = _G.datamile.compile.tracks
_G.datamile.compile.t = _G.datamile.compile.tracks

-- Defaults for the fields every consumer reads. This file runs before the user's
-- init.lua, which re-registers a name to replace the default wholesale.
local function album_value(m, name)
    return d.get(m, "metadata.album", name)
end

local function track_value(m, i, name)
    return d.get(m, "metadata.tracks", i, name)
end

_G.datamile.compile.album.tag("albumartist", function(ctx, m)
    return d.fn.coalesce(album_value(m, "albumartist"), album_value(m, "artist"))
end)
_G.datamile.compile.album.tag("album", function(ctx, m)
    return album_value(m, "album")
end)
_G.datamile.compile.album.tag("date", function(ctx, m)
    return album_value(m, "date")
end)

_G.datamile.compile.tracks.tag("tracknumber", function(ctx, m, i)
    return track_value(m, i, "tracknumber")
end)
_G.datamile.compile.tracks.tag("discnumber", function(ctx, m, i)
    return track_value(m, i, "discnumber")
end)
_G.datamile.compile.tracks.tag("title", function(ctx, m, i)
    return track_value(m, i, "title")
end)
_G.datamile.compile.tracks.tag("artist", function(ctx, m, i)
    return d.fn.coalesce(
        track_value(m, i, "artist"),
        album_value(m, "albumartist"),
        album_value(m, "artist")
    )
end)

_G.datamile.compile.album.image("main", function(ctx, m)
    local root = ctx.paths.album_root
    local candidates = {
        "cover.png",
        "cover.jpg",
        "cover.jpeg",
        "folder.png",
        "folder.jpg",
        "folder.jpeg",
    }
    local matched = nil

    for _, name in ipairs(candidates) do
        if d.fs.exists(d.fs.joinpath(root, name)) then
            matched = name
            break
        end
    end

    return {
        fit = { "crop", 1080, 1080 },
        filter = "catmullrom",
        sources = matched
    }
end)

function __DATAMILE_DISPATCHER(ctx, manifests)
    local results = { id = nil, tags = {}, objects = {}, tracks = {}, images = {} }

    local meta_album = manifests.metadata and manifests.metadata.album or {}
    if type(REGISTRY.id_fn) == "function" then
        local status, res = pcall(REGISTRY.id_fn, ctx, manifests)
        if not status then
            error(string.format("Error evaluating album id: %s", res))
        end
        if res == nil or res == "" then
            error("Album id function returned empty or nil value")
        end
        results.id = tostring(res)
    else
        local artist = d.fn.coalesce(meta_album.albumartist, meta_album.artist, "Unknown")
        local raw_date = tostring(d.fn.coalesce(meta_album.date, ""))
        local year = raw_date:match("%d%d%d%d") or "Unknown"
        local album = d.fn.coalesce(meta_album.album, "Unknown")
        results.id = string.format("%s - %s - %s", artist, year, album)
    end

    for tag_name, func in pairs(REGISTRY.tags.album) do
        local status, res = pcall(func, ctx, manifests)
        if not status then
            error(string.format("Error evaluating album tag '%s': %s", tag_name, res))
        end
        local value = check_tag("album", tag_name, res)
        if value ~= nil then
            results.tags[tag_name] = value
        end
    end

    for name, func in pairs(REGISTRY.objects.album) do
        local status, res = pcall(func, ctx, manifests)
        if not status then
            error(string.format("Error evaluating album object '%s': %s", name, res))
        end
        local value = check_object(res)
        if value ~= nil then
            results.objects[name] = value
        end
    end

    for name, func in pairs(REGISTRY.images_rules) do
        local status, res = pcall(func, ctx, manifests)
        if not status then
            error(string.format("Error evaluating album image '%s': %s", name, res))
        end
        if res ~= nil then
            results.images[name] = res
        end
    end

    for i = 1, ctx.total_tracks do
        results.tracks[i] = { tags = {}, objects = {} }

        for tag_name, func in pairs(REGISTRY.tags.tracks) do
            local status, res = pcall(func, ctx, manifests, i)
            if not status then
                error(string.format("Error evaluating track tag '%s' at index %d: %s", tag_name, i, res))
            end
            local value = check_tag("track", tag_name, res, i)
            if value ~= nil then
                results.tracks[i].tags[tag_name] = value
            end
        end

        for name, func in pairs(REGISTRY.objects.tracks) do
            local status, res = pcall(func, ctx, manifests, i)
            if not status then
                error(string.format("Error evaluating track object '%s' at index %d: %s", name, i, res))
            end
            local value = check_object(res)
            if value ~= nil then
                results.tracks[i].objects[name] = value
            end
        end
    end

    return results
end
