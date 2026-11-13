local M = {}

function M.audio_extensions()
    local configured = d.get(REGISTRY.config, "compiler.audio_extensions")
        or d.get(REGISTRY.config, "manifest.audio_extensions")
    if type(configured) == "table" and #configured > 0 then
        return configured
    end

    return d.fs.audio_extensions()
end

function M.tracks(path)
    local seeds = {}
    for _, file in ipairs(d.fs.files(path, { depth = 1, ext = M.audio_extensions() })) do
        table.insert(seeds, { path = file, audio = d.fs.read_audio(file) })
    end

    return d.list.sort_by(seeds, function(item)
        return {
            d.str.leading_int(d.get(item.audio, "tags.discnumber")) or 1,
            d.str.leading_int(d.get(item.audio, "tags.tracknumber")) or math.huge,
            item.path,
        }
    end)
end

function M.cover(path)
    local matched = d.fs.first(path, {
        "cover.jpg",
        "cover.png",
        "folder.jpg",
        "folder.png",
    })
    if not matched then
        return nil
    end

    return d.fs.basename(matched)
end

function M.info(tracks)
    local duration = 0
    local discs = {}

    for _, item in ipairs(tracks or {}) do
        duration = duration + (d.get(item, "objects.info.duration_milliseconds") or 0)
        discs[tostring(d.get(item, "tags.discnumber") or 1)] = true
    end

    local total_discs = 0
    for _ in pairs(discs) do
        total_discs = total_discs + 1
    end

    return {
        total_tracks = #(tracks or {}),
        total_discs = math.max(total_discs, 1),
        duration_milliseconds = duration,
        duration_formatted = d.fmt.clock(duration),
    }
end

return M
