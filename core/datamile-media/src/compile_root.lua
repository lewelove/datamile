-- Single-pass root compiler. Every hook runs in this VM; the finished tree
-- crosses into Rust exactly once, after all Lua tables are built.

-- Tags are registered one by one through `entity.tag(name, fn)`.
local function eval_tags(def, ctx)
    local out = {}
    for name, fn in pairs(def.tags or {}) do
        local value = fn(ctx)
        if value ~= nil then
            out[name] = value
        end
    end
    return out
end

local function eval_images(def, ctx)
    local out = {}
    for name, fn in pairs(def.images or {}) do
        local spec = fn(ctx)
        if spec ~= nil then
            out[name] = spec
        end
    end
    return out
end

-- `ctx.objects.other` evaluates `objects.other(ctx)` once; a dependency cycle
-- fails immediately instead of recursing forever.
local function make_objects(defs, ctx)
    local store = {}
    local evaluating = {}

    local proxy = setmetatable({}, {
        __index = function(_, key)
            local cached = rawget(store, key)
            if cached ~= nil then
                return cached
            end
            local fn = (defs or {})[key]
            if fn == nil then
                return nil
            end
            if evaluating[key] then
                error(string.format("cyclic object dependency on '%s'", tostring(key)), 0)
            end
            evaluating[key] = true
            local value = fn(ctx)
            evaluating[key] = nil
            if value ~= nil then
                rawset(store, key, value)
            end
            return value
        end,
        __pairs = function(t)
            for key in pairs(defs or {}) do
                local _ = t[key]
            end
            return next, store, nil
        end
    })

    return proxy
end

local function materialize_objects(proxy, defs)
    local out = {}
    for name in pairs(defs or {}) do
        local value = proxy[name]
        if value ~= nil then
            out[name] = value
        end
    end
    return out
end

local eval_node

-- `def.nodes` lists literal registered node names, in output order.
local function eval_child_nodes(def, ctx, root_dir)
    local out = {}

    for _, name in ipairs(def.nodes or {}) do
        local child = REGISTRY.media_nodes[name]
        if child == nil then
            error(string.format("unknown media.node '%s'", tostring(name)), 0)
        end

        local selected = child.select(ctx)
        local items = {}

        if type(selected) == "table" then
            for _, raw in ipairs(selected) do
                items[#items + 1] = eval_node(child, ctx, raw, root_dir)
            end
        elseif selected ~= nil then
            items[1] = eval_node(child, ctx, selected, root_dir)
        end

        if #items > 0 then
            out[name] = items
        end
    end

    return out
end

eval_node = function(def, parent_ctx, raw, root_dir)
    local ctx = { parent = parent_ctx, item = nil }

    if type(raw) == "string" then
        local located = __DATAMILE_NODE_LOCATION(root_dir, raw)
        if located == nil then
            error(string.format("node select returned a missing path: %s", raw), 0)
        end
        ctx.path = located.path
        ctx.location = located.location
    elseif type(raw) == "table" then
        ctx.path = parent_ctx.path
        ctx.item = raw
    else
        error(string.format("node select returned %s, expected a path or a table", type(raw)), 0)
    end

    if type(def.ctx) == "function" then
        def.ctx(ctx)
    end

    ctx.tags = eval_tags(def, ctx)
    ctx.objects = make_objects(def.objects, ctx)

    local result = {}
    if ctx.location ~= nil then
        result.location = ctx.location
    end
    result.tags = ctx.tags
    result.objects = materialize_objects(ctx.objects, def.objects)
    result.images = eval_images(def, ctx)
    result.nodes = eval_child_nodes(def, ctx, root_dir)

    return result
end

function __DATAMILE_COMPILE_ROOT(media_type, root_dir)
    local def = REGISTRY.media_roots[media_type]
    if def == nil then
        error(string.format("media.root '%s' is not registered", tostring(media_type)), 0)
    end

    local ctx = {
        path = root_dir,
        location = __DATAMILE_DIR_LOCATION(root_dir)
    }

    if type(def.ctx) == "function" then
        def.ctx(ctx)
    end

    ctx.tags = eval_tags(def, ctx)
    ctx.objects = make_objects(def.objects, ctx)

    local id = def.id(ctx)
    if id == nil or id == "" then
        error("root id function returned an empty or nil value", 0)
    end
    ctx.id = tostring(id)

    return {
        id = ctx.id,
        location = ctx.location,
        tags = ctx.tags,
        objects = materialize_objects(ctx.objects, def.objects),
        images = eval_images(def, ctx),
        nodes = eval_child_nodes(def, ctx, root_dir)
    }
end
