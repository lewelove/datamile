_G.datamile = _G.datamile or {}
_G.d = _G.datamile

_G.REGISTRY = {
    config = {},
    covers = {
        master = { size = 1080, filter = "mitchell" },
        targets = {}
    },
    media = {},
    covers_rules = {},
    keys = {
        album = {},
        tracks = {}
    },
    id_fn = nil,
    lyrics = nil,
    interfaces = {},
    actions = {},
    dependencies = {},
    filters = {},
    filters_order = {},
    groupers = {},
    groupers_order = {},
    orders = {},
    orders_order = {},
    libraries = {},
    libraries_order = {},
    shelves = {},
    shelves_order = {},
    cabinets = {},
    cabinets_order = {}
}

if not package.searchpath then
    package.searchpath = function(name, path)
        local sep = package.config:sub(1, 1)
        name = name:gsub("%.", sep)
        for c in package.path:gmatch("[^;]+") do
            local filename = c:gsub("%?", name)
            local f = io.open(filename, "r")
            if f then
                f:close()
                return filename
            end
        end
        return nil, "not found"
    end
end

local original_require = require
_G.require = function(modname)
    local path, _ = package.searchpath(modname, package.path)
    if path then
        REGISTRY.dependencies[path] = true
    end
    return original_require(modname)
end

local original_dofile = dofile
_G.dofile = function(filename)
    if filename then
        REGISTRY.dependencies[filename] = true
    end
    return original_dofile(filename)
end

local original_loadfile = loadfile
_G.loadfile = function(filename, mode, env)
    if filename then
        REGISTRY.dependencies[filename] = true
    end
    return original_loadfile(filename, mode, env)
end

function __DATAMILE_GET_CONFIG() return REGISTRY.config end
function __DATAMILE_GET_COVERS() return REGISTRY.covers end
function __DATAMILE_GET_INTERFACES()
    local result = {}
    for name, intf in pairs(REGISTRY.interfaces or {}) do
        result[name] = {
            label = intf.label or name,
            description = intf.description
        }
    end
    return result
end
function __DATAMILE_GET_ACTIONS()
    local result = {}
    for name, action in pairs(REGISTRY.actions or {}) do
        result[name] = {
            label = action.label or name,
            description = action.description
        }
    end
    return result
end
function __DATAMILE_GET_DEPENDENCIES()
    local deps = {}
    for path, _ in pairs(REGISTRY.dependencies) do table.insert(deps, path) end
    return deps
end
