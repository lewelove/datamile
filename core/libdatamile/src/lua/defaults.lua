_G.datamile = _G.datamile or {}
_G.d = _G.datamile

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

local function count_keys(tbl)
    local count = 0
    for _ in pairs(tbl) do
        count = count + 1
    end

    return count
end

local function audio_extensions()
    local configured = d.get(REGISTRY.config, "compiler.audio_extensions")
        or d.get(REGISTRY.config, "manifest.audio_extensions")

    if type(configured) == "table" and #configured > 0 then
        return configured
    end

    return d.fs.audio_extensions()
end

local function find_tracks(path)
    local accepted = {}
    for _, ext in ipairs(audio_extensions()) do
        accepted["." .. ext:lower()] = true
    end

    return d.fs.find(function(name)
        local ext = name:match("%.([^.]+)$")
        if not ext then
            return false
        end

        return accepted["." .. ext:lower()] == true
    end, { path = path, type = "file" })
end

-- Track numbers may be "03/12"; only the leading integer orders tracks.
local function leading_int(value)
    if type(value) == "number" then
        return math.floor(value)
    end
    if type(value) ~= "string" then
        return nil
    end

    local digits = value:match("^%s*(%d+)")
    return digits and tonumber(digits) or nil
end

-- Filesystem walks are unordered, so tracks are keyed by their audio tags to
-- yield a stable album order: (discnumber, tracknumber, path).
local function harvest_tracks(path)
    local harvested = {}
    for _, file in ipairs(find_tracks(path)) do
        table.insert(harvested, { path = file, audio = d.fs.read_audio(file) })
    end

    table.sort(harvested, function(a, b)
        local disc_a = leading_int(d.get(a.audio, "tags.discnumber")) or 1
        local disc_b = leading_int(d.get(b.audio, "tags.discnumber")) or 1
        if disc_a ~= disc_b then
            return disc_a < disc_b
        end

        local num_a = leading_int(d.get(a.audio, "tags.tracknumber")) or math.huge
        local num_b = leading_int(d.get(b.audio, "tags.tracknumber")) or math.huge
        if num_a ~= num_b then
            return num_a < num_b
        end

        return a.path < b.path
    end)

    return harvested
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

local album = d.media.type("album")
local track = d.media.type("track")

album.lock(function(target)
    return d.fs.joinpath(target.path, "album.lock.json")
end)

-- One tree walk finds every album marker. When the scope points straight at an
-- album directory, only that album is selected.
album.select(function(scope)
    local root = scope.path
    local marker = d.fs.joinpath(root, "album.toml")
    local result = {}

    if d.fs.exists(marker) then
        table.insert(result, { path = root })
        return result
    end

    if not scope.recursive then
        return result
    end

    for rel, kind in d.fs.dir(root, { depth = math.huge }) do
        if kind == "file" and d.fs.basename(rel) == "album.toml" then
            table.insert(result, { path = d.fs.dirname(d.fs.joinpath(root, rel)) })
        end
    end

    table.sort(result, function(a, b)
        return a.path < b.path
    end)

    return result
end)

album.input("manifest", function(ctx)
    return d.fs.read_toml(d.fs.joinpath(ctx.path, "album.toml"))
end)

album.input("virtual", function(ctx)
    local virtual_toml = d.fs.read_toml(d.fs.joinpath(ctx.path, "virtual.toml"))
    return d.get(virtual_toml, "album.virtual") == true
end)

album.id(function(ctx)
    local media_directory = d.get(REGISTRY.config, "storage.media_directory")
        or d.get(REGISTRY.config, "storage.music_directory")
    if not media_directory or media_directory == "" then
        return nil
    end

    return d.fs.relpath(media_directory, ctx.path)
end)

album.location(function(ctx)
    local stat = d.fs.stat(ctx.path)

    return {
        kind = "directory",
        path = ctx.path,
        mtime = stat.mtime,
    }
end)

album.tag("albumartist", function(ctx)
    return d.get(ctx, "manifest.album.albumartist") or d.get(ctx, "manifest.album.artist")
end)

album.tag("album", function(ctx)
    return d.get(ctx, "manifest.album.album")
end)

album.tag("date", function(ctx)
    return d.get(ctx, "manifest.album.date") or d.get(ctx, "manifest.album.release_date")
end)

album.child("tracks", "track", function(ctx)
    if ctx.virtual then
        local album_meta = d.get(ctx, "manifest.album") or {}
        local seeds = {}
        for _, entry in ipairs(d.get(ctx, "manifest.tracks") or {}) do
            table.insert(seeds, { manifest = d.tbl.extend("force", album_meta, entry) })
        end
        return seeds
    end

    local seeds = {}
    for _, item in ipairs(harvest_tracks(ctx.path)) do
        table.insert(seeds, { path = item.path, audio = item.audio })
    end

    return seeds
end)

-- Aggregates over the already compiled ctx.tracks.
album.object("info", function(ctx)
    local tracks = ctx.tracks or {}
    local duration = 0
    local discs = {}

    for _, item in ipairs(tracks) do
        duration = duration + (d.get(item, "objects.info.duration_milliseconds") or 0)
        discs[tostring(d.get(item, "tags.discnumber") or 1)] = true
    end

    return {
        virtual = ctx.virtual,
        total_tracks = #tracks,
        total_discs = math.max(count_keys(discs), 1),
        duration_milliseconds = duration,
        duration_formatted = format_duration(duration),
    }
end)

-- Accepts #RGB, #RGBA, #RRGGBB and #RRGGBBAA only.
local function is_hex_color(value)
    if type(value) ~= "string" then
        return false
    end

    local digits = value:match("^#(%x+)$")
    if not digits then
        return false
    end

    local len = #digits
    return len == 3 or len == 4 or len == 6 or len == 8
end

local function color_list(value)
    if type(value) == "string" then
        value = { value }
    end

    if type(value) ~= "table" then
        return nil
    end

    local colors = {}
    for _, item in ipairs(value) do
        if is_hex_color(item) then
            table.insert(colors, item)
        end
    end

    if #colors == 0 then
        return nil
    end

    return colors
end

-- `theme.toml` carries the cover-derived palette consumed by the background
-- shader. Only valid hex colors are compiled.
album.object("colors", function(ctx)
    local theme = d.fs.read_toml(d.fs.joinpath(ctx.path, "theme.toml"))
    local raw = d.get(theme, "album.colors")
    if type(raw) ~= "table" then
        return nil
    end

    local colors = {}
    colors.background = color_list(raw.background)

    if is_hex_color(raw.foreground) then
        colors.foreground = raw.foreground
    end

    if not colors.background and not colors.foreground then
        return nil
    end

    return colors
end)

album.image("main", function(ctx)
    local matched = find_cover(ctx.path)
    if not matched then
        return nil
    end

    return {
        fit = { "crop", 1080, 1080 },
        filter = "catmullrom",
        sources = matched,
    }
end)

track.select(function(scope)
    local seeds = {}
    for _, item in ipairs(harvest_tracks(scope.path)) do
        table.insert(seeds, { path = item.path, audio = item.audio })
    end

    return seeds
end)

-- Parent selectors may pass `audio`, so a track harvests at most once.
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
