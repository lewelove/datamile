_G.datamile = _G.datamile or _G.d or {}
_G.d = _G.datamile

_G.REGISTRY = {
    config = {},
    interfaces = {},
    actions = {},
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

function __DATAMILE_GET_CONFIG() return REGISTRY.config end
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
