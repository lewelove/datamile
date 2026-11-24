_G.datamile = _G.datamile or {}
_G.d = _G.datamile

REGISTRY.media_roots = REGISTRY.media_roots or {}
REGISTRY.media_roots_order = REGISTRY.media_roots_order or {}
REGISTRY.media_nodes = REGISTRY.media_nodes or {}

-- The only methods a media entity has are tag/object/image. Everything else
-- (init, select, id, playable, nodes) belongs in the body table.
local function attach_methods(def, owner)
    def.tags = def.tags or {}
    def.objects = def.objects or {}
    def.images = def.images or {}

    def.tag = function(name, fn)
        if type(name) ~= "string" or name == "" then
            error(owner .. ": tag name must be a non-empty string")
        end
        if type(fn) ~= "function" then
            error(string.format("%s: tag '%s' must be a function", owner, name))
        end
        def.tags[name] = fn
    end

    def.object = function(name, fn)
        if type(name) ~= "string" or name == "" then
            error(owner .. ": object name must be a non-empty string")
        end
        if type(fn) ~= "function" then
            error(string.format("%s: object '%s' must be a function", owner, name))
        end
        def.objects[name] = fn
    end

    def.image = function(name, fn)
        if type(name) ~= "string" or name == "" then
            error(owner .. ": image name must be a non-empty string")
        end
        if type(fn) ~= "function" then
            error(string.format("%s: image '%s' must be a function", owner, name))
        end
        def.images[name] = fn
    end

    return def
end

local function register_root(name, def)
    if type(name) ~= "string" or name == "" then
        error("media.root: name must be a non-empty string")
    end
    if type(def) ~= "table" then
        error(string.format("media.root '%s': expected table, got %s", name, type(def)))
    end

    table.insert(REGISTRY.media_roots_order, name)
    REGISTRY.media_roots[name] = attach_methods(def, string.format("media.root '%s'", name))
    return REGISTRY.media_roots[name]
end

local function register_node(name, def)
    if type(name) ~= "string" or name == "" then
        error("media.node: name must be a non-empty string")
    end
    if type(def) ~= "table" then
        error(string.format("media.node '%s': expected table, got %s", name, type(def)))
    end
    if REGISTRY.media_nodes[name] ~= nil then
        error(string.format("media.node '%s' is already registered", name))
    end

    REGISTRY.media_nodes[name] = attach_methods(def, string.format("media.node '%s'", name))
    return REGISTRY.media_nodes[name]
end

_G.datamile.media = {
    root = register_root,
    node = register_node
}

-- `nodes` is a list of literal registered node names; there are no aliases.
local function node_defs(def, owner)
    local out = {}
    local seen = {}

    for _, name in ipairs(def.nodes or {}) do
        if type(name) ~= "string" or name == "" then
            error(owner .. ": nodes must list literal node names")
        end
        if seen[name] then
            error(string.format("%s: node '%s' is listed twice", owner, name))
        end
        seen[name] = true

        local child = REGISTRY.media_nodes[name]
        if child == nil then
            error(string.format("%s: unknown media.node '%s'", owner, name))
        end
        out[name] = child
    end

    return out
end

local function node_schema(def, owner, visiting)
    local nodes = {}

    for name, child in pairs(node_defs(def, owner)) do
        if visiting[name] then
            error(string.format("media.node cycle at '%s'", name))
        end
        visiting[name] = true
        nodes[name] = {
            playable = child.playable == true,
            has_select = type(child.select) == "function",
            nodes = node_schema(child, string.format("media.node '%s'", name), visiting),
        }
        visiting[name] = nil
    end

    return nodes
end

-- Data-only view of the schema: functions never cross into Rust.
function __DATAMILE_GET_MEDIA_MANIFEST()
    local roots = {}
    for name, def in pairs(REGISTRY.media_roots or {}) do
        roots[name] = {
            init = def.init,
            has_select = type(def.select) == "function",
            has_id = type(def.id) == "function",
            nodes = node_schema(def, string.format("media.root '%s'", name), {}),
        }
    end
    return {
        roots_order = REGISTRY.media_roots_order or {},
        roots = roots
    }
end

-- Phase 1 discovery for one root type.
function __DATAMILE_MEDIA_SELECT(name, dir)
    local def = REGISTRY.media_roots[name]
    if not def then
        error(string.format("media.root '%s' is not registered", tostring(name)))
    end
    return def.select(dir)
end
