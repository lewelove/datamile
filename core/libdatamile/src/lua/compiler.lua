_G.datamile = _G.datamile or {}
_G.d = _G.datamile

_G.datamile.compile = _G.datamile.compile or {}
_G.datamile.media = _G.datamile.media or {}

-- Prepared contexts wait in Lua between PREPARE and FINALIZE so large inputs
-- never round-trip through JSON while their children compile.
local ACTIVE_CTX = {}
_G.__DATAMILE_ACTIVE_CTX = ACTIVE_CTX
local next_ctx_id = 0

local function require_type(name)
    local t = REGISTRY.media[name]
    if not t then
        error(string.format("unknown media type '%s'", name))
    end

    return t
end

local function is_scalar(value)
    local tv = type(value)
    return tv == "string" or tv == "number" or tv == "boolean"
end

local function validate_tag(value)
    if is_scalar(value) then
        return
    end

    if type(value) ~= "table" then
        error("tag value must be a scalar or a list of scalars")
    end

    if not d.tbl.islist(value) then
        error("tag value must be a scalar or a list of scalars")
    end

    for _, item in ipairs(value) do
        if not is_scalar(item) then
            error("tag list must contain scalars only")
        end
    end
end

local function ctx_label(ctx)
    if type(ctx) == "table" and type(ctx.path) == "string" then
        return ctx.path
    end

    return "<virtual>"
end

local function validate_seed(seed)
    if type(seed) ~= "table" then
        error("media seed must be a table")
    end
    if seed.path ~= nil and type(seed.path) ~= "string" then
        error("media seed path must be a string")
    end

    return seed
end

local function normalize_seeds(result, label)
    if type(result) ~= "table" then
        error(string.format("%s must return a table of seeds", label))
    end

    local seeds = {}
    for _, seed in ipairs(result) do
        seeds[#seeds + 1] = validate_seed(seed)
    end

    return seeds
end

local function sorted_keys(tbl)
    local keys = {}
    for key in pairs(tbl) do
        table.insert(keys, key)
    end
    table.sort(keys)

    return keys
end

local function call_hook(fn, ctx, kind, name, type_name)
    local ok, result = pcall(fn, ctx)
    if not ok then
        error(string.format(
            "media type '%s' %s '%s' failed for %s: %s",
            type_name, kind, name, ctx_label(ctx), result
        ))
    end

    return result
end

local function call_selector(fn, ctx, label)
    local ok, result = pcall(fn, ctx)
    if not ok then
        error(string.format("%s failed for %s: %s", label, ctx_label(ctx), result))
    end

    return normalize_seeds(result, label)
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
        name = name,
        tags = {},
        objects = {},
        images = {},
        inputs = {},
        input_order = {},
        children = {},
        child_order = {},
    }
    REGISTRY.media[name] = t

    local function set_fn(field, fn)
        if type(fn) ~= "function" then
            error(string.format("media type '%s': %s must be a function", name, field))
        end
        t[field] = fn
    end

    local function set_named(bucket, kind, key, fn)
        if type(key) ~= "string" or key == "" then
            error(string.format("media type '%s': %s name must be a non-empty string", name, kind))
        end
        if type(fn) ~= "function" then
            error(string.format("media type '%s': %s '%s' must be a function", name, kind, key))
        end
        t[bucket][key] = fn
    end

    local api = {
        select = function(fn) set_fn("select", fn) end,
        lock = function(fn) set_fn("lock", fn) end,
        id = function(fn) set_fn("id", fn) end,
        location = function(fn) set_fn("location", fn) end,
        tag = function(key, fn) set_named("tags", "tag", key, fn) end,
        object = function(key, fn) set_named("objects", "object", key, fn) end,
        image = function(key, fn) set_named("images", "image", key, fn) end,
        input = function(key, fn)
            if type(key) ~= "string" or key == "" then
                error(string.format("media type '%s': input name must be a non-empty string", name))
            end
            if type(fn) ~= "function" then
                error(string.format("media type '%s': input '%s' must be a function", name, key))
            end
            if t.inputs[key] == nil then
                table.insert(t.input_order, key)
            end
            t.inputs[key] = fn
        end,
        child = function(field, child_type, selector)
            if type(field) ~= "string" or field == "" then
                error(string.format("media type '%s': child field must be a non-empty string", name))
            end
            if type(child_type) ~= "string" or child_type == "" then
                error(string.format("media type '%s': child type must be a non-empty string", name))
            end
            if selector ~= nil and type(selector) ~= "function" then
                error(string.format(
                    "media type '%s': child '%s' selector must be a function or nil",
                    name, field
                ))
            end
            if t.children[field] == nil then
                table.insert(t.child_order, field)
            end
            t.children[field] = { type = child_type, select = selector }
        end,
    }

    api.tags = api.tag
    api.objects = api.object
    api.images = api.image

    _G.datamile.compile[name] = api
    return api
end

function _G.datamile.media.select(name, scope)
    local t = require_type(name)
    if type(t.select) ~= "function" then
        error(string.format("media type '%s' has no select function", name))
    end

    local label = string.format("media type '%s' select", name)
    return call_selector(t.select, validate_seed(scope), label)
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

function __DATAMILE_MEDIA_SELECT(name, scope)
    return _G.datamile.media.select(name, scope)
end

function __DATAMILE_MEDIA_LOCK(name, seed)
    local t = require_type(name)
    if type(t.lock) ~= "function" then
        return nil
    end

    seed = validate_seed(seed)
    local ok, result = pcall(t.lock, seed)
    if not ok then
        error(string.format(
            "media type '%s' lock failed for %s: %s", name, ctx_label(seed), result
        ))
    end
    if result == nil then
        return nil
    end
    if type(result) ~= "string" or result == "" then
        error(string.format("media type '%s' lock must return a path or nil", name))
    end

    return result
end

local function run_prepare(t, name, ctx)
    for _, key in ipairs(t.input_order) do
        ctx[key] = call_hook(t.inputs[key], ctx, "input", key, name)
    end

    local children = {}
    for _, field in ipairs(t.child_order) do
        local spec = t.children[field]
        local child_type = require_type(spec.type)
        local selector = spec.select or child_type.select
        if type(selector) ~= "function" then
            error(string.format(
                "media type '%s' child '%s': type '%s' has no select function",
                name, field, spec.type
            ))
        end

        local label = string.format("media type '%s' child '%s' select", name, field)
        table.insert(children, {
            field = field,
            type = spec.type,
            targets = call_selector(selector, ctx, label),
        })
    end

    return children
end

-- Stage 1: run inputs once, then emit the child targets selected from ctx.
function __DATAMILE_MEDIA_PREPARE(name, seed)
    local t = require_type(name)
    local ctx = {}
    for key, value in pairs(validate_seed(seed)) do
        ctx[key] = value
    end

    next_ctx_id = next_ctx_id + 1
    local ctx_id = next_ctx_id
    ACTIVE_CTX[ctx_id] = ctx

    local ok, children = pcall(run_prepare, t, name, ctx)
    if not ok then
        ACTIVE_CTX[ctx_id] = nil
        error(children)
    end

    return { ctx_id = ctx_id, children = children }
end

-- Stage 2: attach the already compiled children, then run the output boxes.
function __DATAMILE_MEDIA_FINALIZE(name, ctx_id, compiled_children)
    local t = require_type(name)
    local ctx = ACTIVE_CTX[ctx_id]
    ACTIVE_CTX[ctx_id] = nil
    if type(ctx) ~= "table" then
        error(string.format(
            "media type '%s' finalize failed for unknown context %s", name, tostring(ctx_id)
        ))
    end

    if type(compiled_children) == "table" then
        for field, value in pairs(compiled_children) do
            ctx[field] = value
        end
    end

    local out = { id = ctx.path or "", tags = {}, objects = {}, images = {} }
    ctx.tags = out.tags
    ctx.objects = out.objects
    ctx.images = out.images

    for _, key in ipairs(sorted_keys(t.tags)) do
        local result = call_hook(t.tags[key], ctx, "tag", key, name)
        if result ~= nil then
            validate_tag(result)
            ctx.tags[key] = result
        end
    end

    if type(t.location) == "function" then
        local result = call_hook(t.location, ctx, "location", "location", name)
        if result ~= nil then
            ctx.location = result
            out.location = result
        end
    end

    for _, key in ipairs(sorted_keys(t.images)) do
        local result = call_hook(t.images[key], ctx, "image", key, name)
        if result ~= nil then
            ctx.images[key] = result
        end
    end

    if type(t.id) == "function" then
        local result = call_hook(t.id, ctx, "id", "id", name)
        if type(result) == "string" and result ~= "" then
            out.id = result
        end
    end
    ctx.id = out.id

    for _, key in ipairs(sorted_keys(t.objects)) do
        local result = call_hook(t.objects[key], ctx, "object", key, name)
        if result ~= nil then
            ctx.objects[key] = result
        end
    end

    return out
end

function __DATAMILE_MEDIA_DROP(ctx_id)
    ACTIVE_CTX[ctx_id] = nil
end
