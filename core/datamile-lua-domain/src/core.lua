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
local ACTION_MEDIA_ERROR =
    "action '%s': 'media' must be a non-empty string or list of non-empty strings"

-- An action's media scope: nil is generic, a string is one type, a list is
-- several. Empty names and empty lists are declaration mistakes.
local function action_media(name, media)
    if media == nil then
        return nil
    end
    if type(media) == "string" then
        if media == "" then
            error(string.format(ACTION_MEDIA_ERROR, name))
        end
        return { media }
    end
    if type(media) ~= "table" then
        error(string.format(ACTION_MEDIA_ERROR, name))
    end

    local list, seen = {}, {}
    for _, entry in ipairs(media) do
        if type(entry) ~= "string" or entry == "" then
            error(string.format(ACTION_MEDIA_ERROR, name))
        end
        if not seen[entry] then
            seen[entry] = true
            table.insert(list, entry)
        end
    end
    if #list == 0 then
        error(string.format(ACTION_MEDIA_ERROR, name))
    end
    return list
end

local function validate_action_text(name, field, value)
    if value == nil or type(value) == "function" then
        return
    end
    if type(value) ~= "string" or value == "" then
        error(string.format("action '%s': '%s' must be a non-empty string or function", name, field))
    end
end

function __DATAMILE_GET_ACTIONS()
    local result = {}
    for name, action in pairs(REGISTRY.actions or {}) do
        if type(action.run) ~= "function" then
            error(string.format("action '%s': 'run' must be a function", name))
        end
        validate_action_text(name, "label", action.label)
        validate_action_text(name, "description", action.description)
        result[name] = action_media(name, action.media) or {}
    end
    return result
end

-- Evaluates a static string or a `function(media)` presentation callback; a
-- callback must return text, otherwise the config is broken.
local function resolve_action_text(name, field, value, media)
    if value == nil then
        return nil
    end
    if type(value) == "string" then
        return value
    end

    local ok, result = pcall(value, media)
    if not ok or type(result) ~= "string" or result == "" then
        if media == nil then
            error(string.format("action '%s': '%s' callback failed", name, field))
        end
        error(string.format(
            "action '%s': '%s' callback failed for media '%s'",
            name, field, media))
    end
    return result
end

-- `label` and `description` are either static text or a callback per media
-- type; the canonical name stands in when no label is declared.
local function action_presentation(name, action, media)
    return {
        label = resolve_action_text(name, "label", action.label, media) or name,
        description = resolve_action_text(name, "description", action.description, media)
    }
end

function __DATAMILE_RESOLVE_ACTIONS(declarations)
    local result = {}
    for name, media_list in pairs(declarations) do
        local action = REGISTRY.actions[name]
        if action == nil then
            error(string.format("action '%s' is not declared", name))
        end
        -- A generic action keeps the flat pair; a scoped one nests one pair per
        -- media type under `media`.
        if #media_list == 0 then
            result[name] = action_presentation(name, action, nil)
        else
            local presentations = {}
            for _, media in ipairs(media_list) do
                presentations[media] = action_presentation(name, action, media)
            end
            result[name] = { label = name, media = presentations }
        end
    end
    return result
end
