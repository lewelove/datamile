_G.datamile = _G.datamile or {}
_G.d = _G.datamile
_G.datamile.compile = _G.datamile.compile or {}

-- The album compiler owns these registries; the domain core leaves them out.
-- `id_fn` stays nil until `datamile.compile.album.id` registers it.
REGISTRY.images_rules = REGISTRY.images_rules or {}
REGISTRY.tags = REGISTRY.tags or {}
REGISTRY.tags.album = REGISTRY.tags.album or {}
REGISTRY.tags.tracks = REGISTRY.tags.tracks or {}
REGISTRY.objects = REGISTRY.objects or {}
REGISTRY.objects.album = REGISTRY.objects.album or {}
REGISTRY.objects.tracks = REGISTRY.objects.tracks or {}

local function is_scalar(v)
    local t = type(v)
    return t == "string" or t == "number" or t == "boolean"
end

local function describe(owner, kind, name, index)
    if index then
        return string.format("%s %s '%s' at index %d", owner, kind, name, index)
    end
    return string.format("%s %s '%s'", owner, kind, name)
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
                    describe(owner, "tag", name, index), type(value)
                ),
                0
            )
        end
        return value
    end

    if next(value) == nil then
        return nil
    end

    local count = 0
    for k, item in pairs(value) do
        if type(k) ~= "number" or not is_scalar(item) then
            error(
                string.format(
                    "Error evaluating %s: expected a scalar or a list of scalars, got a table with non-scalar content",
                    describe(owner, "tag", name, index)
                ),
                0
            )
        end
        count = count + 1
    end

    -- Holes, 0-based keys and float keys serialize as a JSON object instead of an
    -- array, so the list has to be dense from 1. Counting `pairs` first makes the
    -- walk below safe on tables where `#value` is undefined.
    for i = 1, count do
        if value[i] == nil then
            error(
                string.format(
                    "Error evaluating %s: expected a dense 1-based list of scalars, got a sparse table",
                    describe(owner, "tag", name, index)
                ),
                0
            )
        end
    end

    return value
end

-- JSON has no form for functions, userdata, threads or NaN/inf, and mlua only
-- reports those after the dispatcher returns, with no hint about which hook
-- produced them. Walking the value here keeps the failure attached to its owner.
local function json_problem(value, path)
    local t = type(value)

    if t == "table" then
        for k, item in pairs(value) do
            local key_type = type(k)
            if key_type ~= "string" and key_type ~= "number" then
                return string.format("%s has a %s key", path, key_type)
            end

            local problem = json_problem(item, string.format("%s.%s", path, tostring(k)))
            if problem then
                return problem
            end
        end
        return nil
    end

    if t == "string" or t == "boolean" then
        return nil
    end

    if t == "number" then
        -- NaN never equals itself, and infinities have no JSON representation.
        if value == value and value ~= math.huge and value ~= -math.huge then
            return nil
        end
        return string.format("%s is a non-finite number", path)
    end

    return string.format("%s is a %s", path, t)
end

-- Objects take any JSON. Empty means absent, so nothing is written for it.
local function check_object(owner, name, value, index)
    if value == nil or value == "" then
        return nil
    end
    if type(value) == "table" and next(value) == nil then
        return nil
    end

    local problem = json_problem(value, name)
    if problem then
        error(
            string.format(
                "Error evaluating %s: not representable as JSON: %s",
                describe(owner, "object", name, index), problem
            ),
            0
        )
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

function __DATAMILE_DISPATCHER(ctx, manifests)
    local results = { id = nil, tags = {}, objects = {}, tracks = {}, images = {} }

    if type(REGISTRY.id_fn) ~= "function" then
        error("Album id is not registered: call datamile.compile.album.id(fn) in your config")
    end

    local status, res = pcall(REGISTRY.id_fn, ctx, manifests)
    if not status then
        error(string.format("Error evaluating album id: %s", res))
    end
    if res == nil or res == "" then
        error("Album id function returned empty or nil value")
    end
    results.id = tostring(res)

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
        local value = check_object("album", name, res)
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
            local value = check_object("track", name, res, i)
            if value ~= nil then
                results.tracks[i].objects[name] = value
            end
        end
    end

    return results
end
