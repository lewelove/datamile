_G.datamile = _G.datamile or {}
_G.d = _G.datamile
_G.datamile.compile = _G.datamile.compile or {}
_G.datamile.compile.album = {
    key = function(name, v)
        if type(v) == "function" then
            REGISTRY.keys.album[name] = v
        elseif v == true then
            REGISTRY.keys.album[name] = function(ctx, m)
                return m.metadata and m.metadata.album and m.metadata.album[name]
            end
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
    key = function(name, v)
        if type(v) == "function" then
            REGISTRY.keys.tracks[name] = v
        elseif v == true then
            REGISTRY.keys.tracks[name] = function(ctx, m, i)
                return m.metadata and m.metadata.tracks and m.metadata.tracks[i] and m.metadata.tracks[i][name]
            end
        end
    end,
    lyrics = function(t)
        if type(t) == "function" then
            REGISTRY.lyrics = { text = t, type = "txt" }
        elseif type(t) == "table" then
            REGISTRY.lyrics = {
                text = t.text,
                type = t.type or "txt"
            }
        end
    end
}

_G.datamile.compile.a = _G.datamile.compile.album
_G.datamile.compile.track = _G.datamile.compile.tracks
_G.datamile.compile.t = _G.datamile.compile.tracks

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
    local results = { id = nil, album = {}, tracks = {}, images = {} }

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

    for key_name, func in pairs(REGISTRY.keys.album) do
        local status, res = pcall(func, ctx, manifests)
        if not status then
            error(string.format("Error evaluating album key '%s': %s", key_name, res))
        end
        if res ~= nil and res ~= "" then
            results.album[key_name] = res
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
        results.tracks[i] = {}
        for key_name, func in pairs(REGISTRY.keys.tracks) do
            local status, res = pcall(func, ctx, manifests, i)
            if not status then
                error(string.format("Error evaluating track key '%s' at index %d: %s", key_name, i, res))
            end
            if res ~= nil and res ~= "" then
                results.tracks[i][key_name] = res
            end
        end

        if REGISTRY.lyrics and type(REGISTRY.lyrics.text) == "function" then
            local status, lyr_text = pcall(REGISTRY.lyrics.text, ctx, manifests, i)
            if not status then
                error(string.format("Error evaluating track lyrics at index %d: %s", i, lyr_text))
            end
            if lyr_text ~= nil and lyr_text ~= "" then
                local lyr_type = "txt"
                if type(REGISTRY.lyrics.type) == "function" then
                    local t_status, t_res = pcall(REGISTRY.lyrics.type, ctx, manifests, i)
                    if t_status and t_res and t_res ~= "" then
                        lyr_type = t_res
                    end
                elseif type(REGISTRY.lyrics.type) == "string" and REGISTRY.lyrics.type ~= "" then
                    lyr_type = REGISTRY.lyrics.type
                end
                results.tracks[i]["lyrics"] = {
                    ["type"] = lyr_type,
                    ["text"] = lyr_text
                }
            end
        end
    end
    
    return results
end
