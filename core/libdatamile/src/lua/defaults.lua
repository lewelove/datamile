_G.datamile = _G.datamile or {}
_G.d = _G.datamile

local function read_manifest(path)
    local album_toml = d.fs.joinpath(path, "album.toml")
    if d.fs.exists(album_toml) then
        return d.fs.read_toml(album_toml)
    end
    return nil
end

local function is_virtual(path)
    local virtual_toml = d.fs.joinpath(path, "virtual.toml")
    if not d.fs.exists(virtual_toml) then
        return false
    end

    return d.get(d.fs.read_toml(virtual_toml), "album.virtual") == true
end

local function format_duration(ms)
    local total_secs = math.floor((tonumber(ms) or 0) / 1000)
    local hours = math.floor(total_secs / 3600)
    local mins = math.floor((total_secs % 3600) / 60)
    local secs = total_secs - (hours * 3600) - (mins * 60)

    if hours > 0 then
        return string.format("%d:%02d:%02d", hours, mins, secs)
    end

    return string.format("%d:%02d", mins, secs)
end

local function find_albums(root_source)
    local result = {}
    local seen = {}
    local root = root_source.path

    for _, marker in ipairs({ "album.toml" }) do
        for _, manifest in ipairs(d.fs.find(marker, { path = root, type = "file" })) do
            local dir = d.fs.dirname(manifest)
            local nested = false

            for _, other in ipairs(d.fs.find(marker, { path = dir, type = "file" })) do
                if d.fs.dirname(other) ~= dir then
                    nested = true
                    break
                end
            end

            if not nested and not seen[dir] then
                seen[dir] = true
                table.insert(result, { path = dir })
            end
        end
    end

    table.sort(result, function(a, b)
        return a.path < b.path
    end)
    return result
end

local function find_tracks(path)
    return d.fs.find(function(name)
        return name:match("%.flac$") ~= nil
            or name:match("%.mp3$") ~= nil
            or name:match("%.FLAC$") ~= nil
            or name:match("%.MP3$") ~= nil
    end, { path = path, type = "file" })
end

local function find_cover(path)
    for _, name in ipairs({
        "cover.png",
        "cover.jpg",
        "cover.jpeg",
        "folder.png",
        "folder.jpg",
        "folder.jpeg",
    }) do
        local candidate = d.fs.joinpath(path, name)
        if d.fs.exists(candidate) then
            return name
        end
    end

    return nil
end

local function track_sources(album)
    local path = album.path
    if not path then
        return {}
    end

    local sources = {}

    if not is_virtual(path) then
        local files = find_tracks(path)
        if #files > 0 then
            for _, file in ipairs(files) do
                table.insert(sources, { path = file })
            end
            return sources
        end
    end

    local manifest = read_manifest(path)
    local album_data = d.get(manifest, "album") or {}

    for _, entry in ipairs(d.get(manifest, "tracks") or {}) do
        table.insert(sources, {
            data = d.tbl.extend("force", album_data, entry),
        })
    end

    return sources
end

local function track_tag(source, key)
    local value = d.get(source, "data." .. key)
    if value ~= nil then
        return value
    end

    if not source.path then
        return nil
    end

    return d.get(d.fs.read_audio(source.path), "tags." .. key)
end

d.media.type("album")

d.compile.album.select(find_albums)
d.compile.album.id(function(source)
    return d.fs.relpath(REGISTRY.config.storage.music_directory, source.path)
end)
d.compile.album.lock(function(source)
    return d.fs.joinpath(source.path, "album.lock.json")
end)
d.compile.album.location(function(source)
    if not source.path then
        return nil
    end

    local stat = d.fs.stat(source.path)

    return {
        kind = "directory",
        path = source.path,
        mtime = stat.mtime,
    }
end)

d.compile.album.tag("albumartist", function(source)
    local m = read_manifest(source.path)
    return d.get(m, "album.albumartist") or d.get(m, "album.artist")
end)
d.compile.album.tag("album", function(source)
    local m = read_manifest(source.path)
    return d.get(m, "album.album")
end)
d.compile.album.tag("date", function(source)
    local m = read_manifest(source.path)
    return d.get(m, "album.date") or d.get(m, "album.release_date")
end)

d.compile.album.object("info", function(source)
    local tracks = d.media.select("track", source)
    local duration = 0
    local discs = {}

    for _, track in ipairs(tracks) do
        local disc = d.get(track, "data.discnumber")
        if not disc and track.path then
            disc = d.get(d.fs.read_audio(track.path), "tags.discnumber")
        end
        discs[tostring(disc or "1")] = true

        if track.path then
            duration = duration + (d.fs.read_audio(track.path).duration_milliseconds or 0)
        end
    end

    local total_discs = 0
    for _ in pairs(discs) do
        total_discs = total_discs + 1
    end

    return {
        virtual = is_virtual(source.path),
        total_tracks = #tracks,
        total_discs = math.max(total_discs, 1),
        duration_milliseconds = duration,
        duration_formatted = format_duration(duration),
    }
end)

d.compile.album.images("main", function(source)
    local matched = find_cover(source.path)
    if not matched then
        return nil
    end

    return {
        fit = { "crop", 1080, 1080 },
        filter = "catmullrom",
        sources = matched,
    }
end)

d.compile.album.media("tracks", "track")

d.media.type("track")

d.compile.track.select(track_sources)
d.compile.track.id(function(_, tags)
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
d.compile.track.location(function(source)
    if not source.path then
        return nil
    end

    local stat = d.fs.stat(source.path)

    return {
        kind = "file",
        path = source.path,
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
    d.compile.track.tag(key, function(source)
        return track_tag(source, key)
    end)
end

d.compile.track.object("info", function(source)
    if not source.path then
        return {
            sample_rate = 0,
            bits_per_sample = 0,
            bitrate_kbps = 0,
            encoding = "virtual",
            channels = 0,
            duration_milliseconds = 0,
            duration_formatted = "0:00",
        }
    end

    local audio = d.fs.read_audio(source.path)
    return {
        sample_rate = audio.sample_rate,
        bits_per_sample = audio.bit_depth or 0,
        bitrate_kbps = audio.audio_bitrate or 0,
        encoding = audio.format,
        channels = audio.channels or 0,
        duration_milliseconds = audio.duration_milliseconds or 0,
        duration_formatted = format_duration(audio.duration_milliseconds or 0),
    }
end)
