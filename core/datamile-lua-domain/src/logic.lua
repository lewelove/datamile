_G.dl = _G.dl or {}

-- A logic unit without `media` predates the single-root rule; during the
-- transition it is treated as an album definition.
local function unit_media(def)
    if def.media == nil or def.media == "" then
        return "album"
    end
    return def.media
end

local function applies(def, media_type)
    return unit_media(def) == media_type
end

function __DATAMILE_GET_LOGIC_MANIFEST()
    local manifest = {
        filters = {},
        groupers = {},
        orders = {},
        libraries = {},
        shelves = {},
        cabinets = {},
        filters_order = REGISTRY.filters_order or {},
        groupers_order = REGISTRY.groupers_order or {},
        orders_order = REGISTRY.orders_order or {},
        libraries_order = REGISTRY.libraries_order or {},
        shelves_order = REGISTRY.shelves_order or {},
        cabinets_order = REGISTRY.cabinets_order or {}
    }
    for k, v in pairs(REGISTRY.filters or {}) do
        manifest.filters[k] = { media = unit_media(v), label = v.label or k }
    end
    for k, v in pairs(REGISTRY.groupers or {}) do
        manifest.groupers[k] = {
            media = unit_media(v),
            label = v.label or k,
            index = v.index,
            count = v.count,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.orders or {}) do
        manifest.orders[k] = {
            media = unit_media(v),
            label = v.label or k,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.libraries or {}) do
        manifest.libraries[k] = {
            media = unit_media(v),
            label = v.label or k,
            filters = v.filters or {},
            groupers = v.groupers or {},
            orders = v.orders or {}
        }
    end
    for k, v in pairs(REGISTRY.shelves or {}) do
        manifest.shelves[k] = {
            media = unit_media(v),
            label = v.label or k,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.cabinets or {}) do
        manifest.cabinets[k] = {
            media = unit_media(v),
            label = v.label or k,
            shelves = v.shelves or {},
            orders = v.orders or {}
        }
    end
    return manifest
end

-- Evaluates only the units declared for `media_type`, so two media types can
-- reuse ids without leaking matches into each other.
function __DATAMILE_EVALUATE_LOGIC(media_type, entity)
    local res = {
        filters = {},
        groupers = {},
        orders = {},
        libraries = {},
        shelves = {},
        shelf_sorts = {}
    }

    for k, v in pairs(REGISTRY.filters or {}) do
        if applies(v, media_type) and type(v.match) == "function" then
            local ok, match_res = pcall(v.match, entity)
            if ok and match_res then
                res.filters[k] = true
            elseif not ok then
                error(string.format("Error evaluating filter '%s': %s", k, tostring(match_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.groupers or {}) do
        if applies(v, media_type) and type(v.select) == "function" then
            local ok, sel_res = pcall(v.select, entity)
            if ok and sel_res ~= nil then
                if type(sel_res) == "table" then
                    local items = {}
                    for _, item in ipairs(sel_res) do
                        if item ~= nil and item ~= "" then
                            table.insert(items, tostring(item))
                        end
                    end
                    res.groupers[k] = items
                elseif sel_res ~= "" then
                    res.groupers[k] = { tostring(sel_res) }
                end
            elseif not ok then
                error(string.format("Error evaluating grouper '%s': %s", k, tostring(sel_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.orders or {}) do
        if applies(v, media_type) and type(v.sort) == "function" then
            local ok, sort_res = pcall(v.sort, entity)
            if ok and sort_res ~= nil then
                res.orders[k] = sort_res
            elseif not ok then
                error(string.format("Error evaluating order '%s': %s", k, tostring(sort_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.libraries or {}) do
        if applies(v, media_type) and type(v.match) == "function" then
            local ok, match_res = pcall(v.match, entity)
            if ok and match_res then
                res.libraries[k] = true
            elseif not ok then
                error(string.format("Error evaluating library '%s': %s", k, tostring(match_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.shelves or {}) do
        if applies(v, media_type) then
            if type(v.match) == "function" then
                local ok, match_res = pcall(v.match, entity)
                if ok and match_res then
                    res.shelves[k] = true
                elseif not ok then
                    error(string.format("Error evaluating shelf match '%s': %s", k, tostring(match_res)))
                end
            end
            if type(v.sort) == "function" then
                local ok, sort_res = pcall(v.sort, entity)
                if ok and sort_res ~= nil then
                    res.shelf_sorts[k] = sort_res
                elseif not ok then
                    error(string.format("Error evaluating shelf sort '%s': %s", k, tostring(sort_res)))
                end
            end
        end
    end

    return res
end
