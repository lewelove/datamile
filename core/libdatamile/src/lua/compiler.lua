_G.datamile = _G.datamile or {}
_G.d = _G.datamile

_G.datamile.compile = _G.datamile.compile or {}
_G.datamile.media = _G.datamile.media or {}

-- Prepared contexts wait in Lua between PREPARE and FINALIZE so large inputs
-- never round-trip through JSON while their children compile.
local ACTIVE_CTX = {}
_G.__DATAMILE_ACTIVE_CTX = ACTIVE_CTX
local next_ctx_id = 0

local ROOT_KIND = "root"
local CHILD_KINDS = { dir = true, file = true, virtual = true }
local RESERVED_SEED_KEYS = {
    path = true,
    parent = true,
    id = true,
    tags = true,
    objects = true,
    files = true,
    images = true,
}

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

local function is_absolute(path)
    return path:sub(1, 1) == "/" or path:match("^%a:[/\\]") ~= nil
end

local function resolve_dir(raw)
    if type(raw) ~= "string" or raw == "" then
        error("media type dir must be a non-empty string")
    end

    local cwd = REGISTRY.config_dir
    local resolved = cwd and d.fs.abspath(raw, { cwd = cwd }) or raw
    return d.fs.normalize(resolved)
end

local function normalize_kind(kind, name)
    if type(kind) == "string" then
        if kind == ROOT_KIND then
            return { is_root = true, kinds = { ROOT_KIND } }
        end
        if not CHILD_KINDS[kind] then
            error(string.format("media type '%s': unknown kind '%s'", name, tostring(kind)))
        end

        return { is_root = false, kinds = { kind } }
    end

    if type(kind) ~= "table" then
        error(string.format("media type '%s': kind is required", name))
    end

    local kinds = {}
    local seen = {}
    for _, item in ipairs(kind) do
        if type(item) ~= "string" then
            error(string.format("media type '%s': kind entries must be strings", name))
        end
        if item == ROOT_KIND then
            error(string.format("media type '%s': 'root' cannot be combined", name))
        end
        if not CHILD_KINDS[item] then
            error(string.format("media type '%s': unknown kind '%s'", name, item))
        end
        if seen[item] then
            error(string.format("media type '%s': duplicate kind '%s'", name, item))
        end
        seen[item] = true
        table.insert(kinds, item)
    end

    if #kinds == 0 then
        error(string.format("media type '%s': kind list cannot be empty", name))
    end

    return { is_root = false, kinds = kinds }
end

local function normalize_children(children, name)
    if children == nil then
        return {}, {}
    end
    if type(children) ~= "table" then
        error(string.format("media type '%s': children must be a table", name))
    end

    local order = {}
    for field, child_type in pairs(children) do
        if type(field) ~= "string" or field == "" then
            error(string.format("media type '%s': child field must be a non-empty string", name))
        end
        if type(child_type) ~= "string" or child_type == "" then
            error(string.format("media type '%s': child '%s' must name a media type", name, field))
        end
        table.insert(order, field)
    end
    table.sort(order)

    return children, order
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

    if type(result) ~= "table" then
        error(string.format("%s must return a table of targets", label))
    end

    local targets = {}
    for _, target in ipairs(result) do
        local tv = type(target)
        if tv ~= "string" and tv ~= "table" then
            error(string.format("%s returned an invalid target (%s)", label, tv))
        end
        table.insert(targets, target)
    end

    return targets
end

local function has_kind(t, kind)
    for _, item in ipairs(t.kinds) do
        if item == kind then
            return true
        end
    end

    return false
end

local function is_physical_file(t, ctx)
    if not has_kind(t, "file") then
        return false
    end
    if not has_kind(t, "dir") then
        return true
    end

    local stat = d.fs.stat(ctx.path)
    return stat ~= nil and stat.is_file == true
end

local function entity_dir(t, ctx)
    if type(ctx.path) == "string" then
        if is_physical_file(t, ctx) then
            return d.fs.dirname(ctx.path)
        end
        return ctx.path
    end

    local parent = ctx.parent
    if type(parent) == "table" and type(parent.path) == "string" then
        local stat = d.fs.stat(parent.path)
        if stat and stat.is_dir then
            return parent.path
        end
        return d.fs.dirname(parent.path)
    end

    return nil
end

local function resolve_file_path(t, ctx, raw)
    if type(raw) ~= "string" or raw == "" then
        error("file hook must return a path string or a list of path strings")
    end
    if is_absolute(raw) then
        return raw
    end

    local base = entity_dir(t, ctx)
    if not base then
        error(string.format("cannot resolve relative file path '%s' for virtual entity", raw))
    end

    return d.fs.joinpath(base, raw)
end

local function stat_file(t, ctx, raw)
    local path = resolve_file_path(t, ctx, raw)
    local stat = d.fs.stat(path)
    if not stat or not stat.is_file then
        return nil
    end

    return { path = path, mtime = stat.mtime, byte_size = stat.byte_size }
end

local function collect_files(t, ctx, name, result)
    if type(result) == "string" then
        local item = stat_file(t, ctx, result)
        if item then
            ctx.files[name] = item
        end
        return
    end
    if type(result) ~= "table" then
        error("file hook must return a path string, a list of path strings, or nil")
    end

    local items = {}
    for _, raw in ipairs(result) do
        local item = stat_file(t, ctx, raw)
        if item then
            table.insert(items, item)
        end
    end
    if #items > 0 then
        ctx.files[name] = items
    end
end

local function validate_registry()
    for name, t in pairs(REGISTRY.media) do
        if t.is_root then
            if not t.dir or t.dir == "" then
                error(string.format("root media type '%s' requires a dir", name))
            end
            if type(t.select) ~= "function" then
                error(string.format("root media type '%s' requires a select function", name))
            end
        elseif type(t.select) ~= "function" then
            error(string.format("media type '%s' requires a select function", name))
        end

        for field, child_name in pairs(t.children) do
            local child = REGISTRY.media[child_name]
            if not child then
                error(string.format(
                    "media type '%s' child '%s' references unknown type '%s'",
                    name, field, child_name
                ))
            end
            if child.is_root then
                error(string.format(
                    "media type '%s' child '%s' cannot reference root type '%s'",
                    name, field, child_name
                ))
            end

            local physical_parent = has_kind(t, "dir") or t.is_root
            if not physical_parent and not has_kind(child, "virtual") then
                error(string.format(
                    "media type '%s' can only have virtual children, but '%s' is not virtual",
                    name, child_name
                ))
            end
            if not physical_parent then
                for _, kind in ipairs(child.kinds) do
                    if kind ~= "virtual" then
                        error(string.format(
                            "media type '%s' can only have virtual children, but '%s' declares '%s'",
                            name, child_name, kind
                        ))
                    end
                end
            end
        end
    end

    return true
end

function _G.datamile.media(name, spec)
    if type(name) ~= "string" or name == "" then
        error("media type name must be a non-empty string")
    end
    if type(spec) ~= "table" then
        error(string.format("media type '%s': spec must be a table", name))
    end

    local existing = REGISTRY.media[name]
    if existing then
        return existing.handle
    end

    local normalized = normalize_kind(spec.kind, name)
    local children, child_order = normalize_children(spec.children, name)
    if type(spec.select) ~= "function" then
        error(string.format("media type '%s': select must be a function", name))
    end

    local t = {
        name = name,
        is_root = normalized.is_root,
        kinds = normalized.kinds,
        dir = normalized.is_root and resolve_dir(spec.dir) or nil,
        select = spec.select,
        children = children,
        child_order = child_order,
        tags = {},
        objects = {},
        images = {},
        files = {},
        inputs = {},
        input_order = {},
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
        id = function(fn) set_fn("id", fn) end,
        tag = function(key, fn) set_named("tags", "tag", key, fn) end,
        object = function(key, fn) set_named("objects", "object", key, fn) end,
        image = function(key, fn) set_named("images", "image", key, fn) end,
        file = function(key, fn) set_named("files", "file", key, fn) end,
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
    }

    api.tags = api.tag
    api.objects = api.object
    api.images = api.image
    api.files = api.file

    t.handle = api
    _G.datamile.compile[name] = api
    return api
end

local function select_targets(name, dir_or_scope)
    local t = require_type(name)
    if type(t.select) ~= "function" then
        error(string.format("media type '%s' has no select function", name))
    end

    local scope = dir_or_scope
    if t.is_root and type(scope) == "table" and type(scope.path) == "string" then
        scope = scope.path
    end

    local label = string.format("media type '%s' select", name)
    return call_selector(t.select, scope, label)
end

function __DATAMILE_MEDIA_ROOTS()
    local roots = {}
    for name, t in pairs(REGISTRY.media) do
        if t.is_root then
            table.insert(roots, name)
        end
    end
    table.sort(roots)

    return roots
end

function __DATAMILE_MEDIA_ROOT_DIRS()
    local roots = {}
    for _, name in ipairs(__DATAMILE_MEDIA_ROOTS()) do
        local t = REGISTRY.media[name]
        table.insert(roots, { name = name, dir = t.dir })
    end

    return roots
end

function __DATAMILE_MEDIA_INFO(name)
    local t = require_type(name)
    return {
        is_root = t.is_root,
        kinds = t.kinds,
        dir = t.dir,
    }
end

function __DATAMILE_MEDIA_VALIDATE()
    return validate_registry()
end

function __DATAMILE_MEDIA_SELECT(name, scope)
    return select_targets(name, scope)
end

function __DATAMILE_MEDIA_LOCK(name, seed)
    local t = require_type(name)
    if not t.is_root then
        return nil
    end

    seed = validate_seed(seed)
    return d.fs.joinpath(seed.path, name .. ".lock.json")
end

local function run_prepare(t, name, ctx)
    for _, key in ipairs(t.input_order) do
        ctx[key] = call_hook(t.inputs[key], ctx, "input", key, name)
    end

    local children = {}
    for _, field in ipairs(t.child_order) do
        local child_name = t.children[field]
        local child_type = require_type(child_name)
        if type(child_type.select) ~= "function" then
            error(string.format(
                "media type '%s' child '%s': type '%s' has no select function",
                name, field, child_name
            ))
        end

        local label = string.format("media type '%s' child '%s' select", name, field)
        table.insert(children, {
            field = field,
            type = child_name,
            targets = call_selector(child_type.select, ctx, label),
        })
    end

    return children
end

-- Stage 1: run inputs once, then emit the child targets selected from ctx.
function __DATAMILE_MEDIA_PREPARE(name, seed, parent_ctx_id)
    local t = require_type(name)
    local ctx = {}
    for key, value in pairs(validate_seed(seed)) do
        ctx[key] = value
    end

    if parent_ctx_id ~= nil then
        local parent = ACTIVE_CTX[parent_ctx_id]
        if type(parent) ~= "table" then
            error(string.format("media type '%s': parent context %s is not active", name, tostring(parent_ctx_id)))
        end
        ctx.parent = parent
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

    local out = { id = "", tags = {}, objects = {}, images = {}, files = {} }
    ctx.tags = out.tags
    ctx.objects = out.objects
    ctx.images = out.images
    ctx.files = out.files

    for _, key in ipairs(sorted_keys(t.files)) do
        local result = call_hook(t.files[key], ctx, "file", key, name)
        if result ~= nil then
            collect_files(t, ctx, key, result)
        end
    end

    for _, key in ipairs(sorted_keys(t.images)) do
        local result = call_hook(t.images[key], ctx, "image", key, name)
        if result ~= nil then
            ctx.images[key] = result
        end
    end

    for _, key in ipairs(sorted_keys(t.tags)) do
        local result = call_hook(t.tags[key], ctx, "tag", key, name)
        if result ~= nil then
            validate_tag(result)
            ctx.tags[key] = result
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
