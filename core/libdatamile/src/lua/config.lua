_G.datamile = _G.datamile or {}
_G.d = _G.datamile

_G.datamile.config = function(t)
    REGISTRY.config = t
end

_G.datamile.interface = function(name, t)
    if type(t) ~= "table" then
        error(string.format("interface '%s': expected table, got %s", tostring(name), type(t)))
    end
    local run_fn = t.run
    if type(run_fn) ~= "function" then
        error(string.format("interface '%s': missing 'run' function", tostring(name)))
    end
    REGISTRY.interfaces[name] = {
        run = run_fn,
        label = t.label or name,
        description = t.description
    }
end

_G.datamile.action = function(name, t)
    if type(t) ~= "table" then
        error(string.format("action '%s': expected table, got %s", tostring(name), type(t)))
    end
    local existing = REGISTRY.actions[name] or {}
    local run_fn = t.run or existing.run
    if type(run_fn) ~= "function" then
        error(string.format("action '%s': missing 'run' function", tostring(name)))
    end
    REGISTRY.actions[name] = {
        run = run_fn,
        config = t.config or existing.config or {}
    }
end

_G.datamile.cache = _G.datamile.cache or {}

_G.datamile.cache.cover = function(t)
    table.insert(REGISTRY.covers.targets, t)
end

_G.datamile.cache.master = {
    cover = function(t)
        if t.size then REGISTRY.covers.master.size = t.size end
        if t.filter then REGISTRY.covers.master.filter = t.filter end
    end
}

_G.datamile.cache.image = _G.datamile.cache.cover
_G.datamile.cache.master.image = _G.datamile.cache.master.cover

_G.datamile.filter = function(name, t)
    if REGISTRY.filters[name] == nil then
        table.insert(REGISTRY.filters_order, name)
    end
    REGISTRY.filters[name] = t
end

_G.datamile.grouper = function(name, t)
    if REGISTRY.groupers[name] == nil then
        table.insert(REGISTRY.groupers_order, name)
    end
    REGISTRY.groupers[name] = t
end

_G.datamile.order = function(name, t)
    if REGISTRY.orders[name] == nil then
        table.insert(REGISTRY.orders_order, name)
    end
    REGISTRY.orders[name] = t
end

_G.datamile.library = function(name, t)
    if REGISTRY.libraries[name] == nil then
        table.insert(REGISTRY.libraries_order, name)
    end
    REGISTRY.libraries[name] = t
end

_G.datamile.shelf = function(name, t)
    if REGISTRY.shelves[name] == nil then
        table.insert(REGISTRY.shelves_order, name)
    end
    REGISTRY.shelves[name] = t
end

_G.datamile.cabinet = function(name, t)
    if REGISTRY.cabinets[name] == nil then
        table.insert(REGISTRY.cabinets_order, name)
    end
    REGISTRY.cabinets[name] = t
end
