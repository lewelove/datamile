_G.datamile = _G.datamile or {}
_G.d = _G.datamile

_G.datamile.compile = _G.datamile.compile or {}
_G.datamile.media = _G.datamile.media or {}

local function require_type(name)
    if type(name) ~= "string" or name == "" then
        error("media type name must be a non-empty string")
    end

    local t = REGISTRY.media[name]
    if not t then
        error(string.format("unknown media type '%s'", name))
    end

    return t
end

local function validate_tag(value)
    local function scalar(v)
        local tv = type(v)
        return tv == "string" or tv == "number" or tv == "boolean"
    end

    if scalar(value) then
        return
    end

    if type(value) ~= "table" then
        error("tag value must be a scalar or a list of scalars")
    end

    if not d.tbl.islist(value) then
        error("tag value must be a scalar or a list of scalars")
    end

    for _, item in ipairs(value) do
        if not scalar(item) then
            error("tag list must contain scalars only")
        end
    end
end

local function source_label(source)
    if type(source) == "table" and type(source.path) == "string" then
        return source.path
    end

    return "<virtual>"
end

local function validate_source(source)
    if type(source) ~= "table" then
        error("media source must be a table")
    end
    if source.path ~= nil and type(source.path) ~= "string" then
        error("media source path must be a string")
    end

    return source
end

local function normalize_sources(result, label)
    if type(result) ~= "table" then
        error(string.format("%s must return a table of sources", label))
    end

    local sources = {}
    for _, source in ipairs(result) do
        sources[#sources + 1] = validate_source(source)
    end

    return sources
end

local function run_selector(fn, source, label)
    local ok, result = pcall(fn, source)
    if not ok then
        error(string.format("%s failed for %s: %s", label, source_label(source), result))
    end

    return normalize_sources(result, label)
end

local function call_field(fn, source, kind, name)
    local ok, result = pcall(fn, source)
    if not ok then
        error(string.format("%s '%s' failed for %s: %s", kind, name, source_label(source), result))
    end
    return result
end

function _G.datamile.media.type(name)
    if type(name) ~= "string" or name == "" then
        error("media type name must be a non-empty string")
    end

    local t = REGISTRY.media[name]
    if t then
        return _G.datamile.compile[name]
    end

    t = {
        tags = {},
        objects = {},
        images = {},
        media = {},
    }
    REGISTRY.media[name] = t

    local api = {
        select = function(fn)
            if type(fn) ~= "function" then
                error(string.format("media type '%s': select must be a function", name))
            end
            t.select = fn
        end,
        id = function(fn)
            if type(fn) ~= "function" then
                error(string.format("media type '%s': id must be a function", name))
            end
            t.id = fn
        end,
        lock = function(fn)
            if type(fn) ~= "function" then
                error(string.format("media type '%s': lock must be a function", name))
            end
            t.lock = fn
        end,
        tag = function(key, fn)
            if type(key) ~= "string" or key == "" then
                error(string.format("media type '%s': tag name must be a non-empty string", name))
            end
            if fn ~= nil and type(fn) ~= "function" then
                error(string.format("media type '%s': tag '%s' must be a function or nil", name, key))
            end
            t.tags[key] = fn
        end,
        object = function(key, fn)
            if type(key) ~= "string" or key == "" then
                error(string.format("media type '%s': object name must be a non-empty string", name))
            end
            if fn ~= nil and type(fn) ~= "function" then
                error(string.format("media type '%s': object '%s' must be a function or nil", name, key))
            end
            t.objects[key] = fn
        end,
        image = function(key, fn)
            if type(key) ~= "string" or key == "" then
                error(string.format("media type '%s': image name must be a non-empty string", name))
            end
            if fn ~= nil and type(fn) ~= "function" then
                error(string.format("media type '%s': image '%s' must be a function or nil", name, key))
            end
            t.images[key] = fn
        end,
        location = function(fn)
            if fn ~= nil and type(fn) ~= "function" then
                error(string.format("media type '%s': location must be a function or nil", name))
            end
            t.location = fn
        end,
        media = function(field, child_type, fn)
            if type(field) ~= "string" or field == "" then
                error(string.format("media type '%s': media field must be a non-empty string", name))
            end
            if type(child_type) ~= "string" or child_type == "" then
                error(string.format("media type '%s': child type must be a non-empty string", name))
            end
            if fn ~= nil and type(fn) ~= "function" then
                error(string.format("media type '%s': media field '%s' selector must be a function or nil", name, field))
            end
            t.media[field] = {
                type = child_type,
                select = fn,
            }
        end,
    }

    api.tags = api.tag
    api.objects = api.object
    api.images = api.image

    _G.datamile.compile[name] = api
    return api
end

function _G.datamile.media.select(name, source)
    local t = require_type(name)
    if type(t.select) ~= "function" then
        error(string.format("media type '%s' has no select function", name))
    end

    local label = string.format("media type '%s' select", name)
    return run_selector(t.select, source, label)
end

function __DATAMILE_MEDIA_ROOTS()
    local roots = {}
    for name, t in pairs(REGISTRY.media) do
        if type(t.lock) == "function" then
            table.insert(roots, name)
        end
    end
    table.sort(roots)
    return roots
end

function __DATAMILE_MEDIA_SELECT(name, source)
    return _G.datamile.media.select(name, source)
end

function __DATAMILE_MEDIA_LOCK(name, source)
    local t = require_type(name)
    if type(t.lock) ~= "function" then
        return nil
    end

    source = validate_source(source)
    local ok, result = pcall(t.lock, source)
    if not ok then
        error(string.format("media type '%s' lock failed for %s: %s", name, source_label(source), result))
    end
    if result == nil then
        return nil
    end
    if type(result) ~= "string" or result == "" then
        error(string.format("media type '%s' lock must return a path or nil", name))
    end

    return result
end

function __DATAMILE_MEDIA_COMPILE(name, source)
    local t = require_type(name)
    source = validate_source(source)

    local out = {
        id = source.path or "",
        tags = {},
        objects = {},
        images = {},
        media = {},
    }

    for key, fn in pairs(t.tags) do
        if type(fn) == "function" then
            local result = call_field(fn, source, "tag", key)
            if result ~= nil then
                validate_tag(result)
                out.tags[key] = result
            end
        end
    end

    if type(t.id) == "function" then
        local ok, result = pcall(t.id, source, out.tags)
        if not ok then
            error(string.format("media type '%s' id failed for %s: %s", name, source_label(source), result))
        end
        if type(result) == "string" and result ~= "" then
            out.id = result
        end
    end

    for key, fn in pairs(t.objects) do
        if type(fn) == "function" then
            local result = call_field(fn, source, "object", key)
            if result ~= nil then
                out.objects[key] = result
            end
        end
    end

    if type(t.location) == "function" then
        local result = call_field(t.location, source, "location", "location")
        if result ~= nil then
            out.location = result
        end
    end

    for key, fn in pairs(t.images) do
        if type(fn) == "function" then
            local result = call_field(fn, source, "image", key)
            if result ~= nil then
                out.images[key] = result
            end
        end
    end

    for field, child in pairs(t.media) do
        local child_type = require_type(child.type)
        local selector = child.select or child_type.select
        if type(selector) ~= "function" then
            error(string.format("media type '%s' field '%s': child type '%s' has no select function", name, field, child.type))
        end

        local label = string.format("media type '%s' field '%s' select", name, field)
        out.media[field] = {
            type = child.type,
            sources = run_selector(selector, source, label),
        }
    end

    return out
end
