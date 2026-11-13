_G.dl = _G.dl or {}

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
        manifest.filters[k] = { label = v.label or k }
    end
    for k, v in pairs(REGISTRY.groupers or {}) do
        manifest.groupers[k] = {
            label = v.label or k,
            index = v.index,
            count = v.count,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.orders or {}) do
        manifest.orders[k] = {
            label = v.label or k,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.libraries or {}) do
        manifest.libraries[k] = {
            label = v.label or k,
            filters = v.filters or {},
            groupers = v.groupers or {},
            orders = v.orders or {}
        }
    end
    for k, v in pairs(REGISTRY.shelves or {}) do
        manifest.shelves[k] = {
            label = v.label or k,
            reverse = v.reverse or false
        }
    end
    for k, v in pairs(REGISTRY.cabinets or {}) do
        manifest.cabinets[k] = {
            label = v.label or k,
            shelves = v.shelves or {},
            orders = v.orders or {}
        }
    end
    return manifest
end

function __DATAMILE_EVALUATE_ALBUM_LOGIC(raw_album)
    local res = {
        filters = {},
        groupers = {},
        orders = {},
        libraries = {},
        shelves = {},
        shelf_sorts = {}
    }

    if type(raw_album) ~= "table" or type(raw_album.album) ~= "table" then
        return res
    end

    local album = raw_album.album

    for k, v in pairs(REGISTRY.filters or {}) do
        if type(v.match) == "function" then
            local ok, match_res = pcall(v.match, album)
            if ok and match_res then
                res.filters[k] = true
            elseif not ok then
                error(string.format("Error evaluating filter '%s': %s", k, tostring(match_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.groupers or {}) do
        if type(v.select) == "function" then
            local ok, sel_res = pcall(v.select, album)
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
        if type(v.sort) == "function" then
            local ok, sort_res = pcall(v.sort, album)
            if ok and sort_res ~= nil then
                res.orders[k] = sort_res
            elseif not ok then
                error(string.format("Error evaluating order '%s': %s", k, tostring(sort_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.libraries or {}) do
        if type(v.match) == "function" then
            local ok, match_res = pcall(v.match, album)
            if ok and match_res then
                res.libraries[k] = true
            elseif not ok then
                error(string.format("Error evaluating library '%s': %s", k, tostring(match_res)))
            end
        end
    end

    for k, v in pairs(REGISTRY.shelves or {}) do
        if type(v.match) == "function" then
            local ok, match_res = pcall(v.match, album)
            if ok and match_res then
                res.shelves[k] = true
            elseif not ok then
                error(string.format("Error evaluating shelf match '%s': %s", k, tostring(match_res)))
            end
        end
        if type(v.sort) == "function" then
            local ok, sort_res = pcall(v.sort, album)
            if ok and sort_res ~= nil then
                res.shelf_sorts[k] = sort_res
            elseif not ok then
                error(string.format("Error evaluating shelf sort '%s': %s", k, tostring(sort_res)))
            end
        end
    end

    return res
end
