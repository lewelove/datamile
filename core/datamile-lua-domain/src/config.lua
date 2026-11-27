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
    if type(name) ~= "string" or name == "" then
        error("action name must be a non-empty string")
    end
    if type(t) ~= "table" then
        error(string.format("action '%s': expected table, got %s", tostring(name), type(t)))
    end
    -- `-` and `_` name the same action everywhere: the registry, `datamile x`
    -- and `POST /api/x` all use the canonical form.
    local canonical = name:gsub("%-", "_")
    local existing = REGISTRY.actions[canonical]
    if existing then
        -- Name the spelling that needed canonicalizing, whichever declaration
        -- carried it.
        local source = name ~= canonical and name or existing.name
        error(string.format(
            "action '%s' is declared twice (canonicalized from '%s')",
            canonical, source))
    end
    if type(t.run) ~= "function" then
        error(string.format("action '%s': missing 'run' function", name))
    end
    -- The declaration is stored raw: shape validation and the presentation
    -- callbacks stay in Lua, which Rust cannot carry over.
    REGISTRY.actions[canonical] = {
        name = name,
        media = t.media,
        label = t.label,
        description = t.description,
        run = t.run
    }
end

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
