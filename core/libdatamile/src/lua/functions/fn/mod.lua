local function is_blank(v)
    if v == nil then return true end
    if type(v) == "string" then
        return v:match("^%s*$") ~= nil
    end
    if type(v) == "table" then
        return next(v) == nil
    end
    return false
end

return {
    present = function(v, msg)
        if is_blank(v) then
            error(msg or "Value must be present and non-empty")
        end
        return v
    end,

    type_check = function(v, t)
        if v == nil or v == "" then
            return nil
        end
        if type(t) ~= "string" then
            error("Type check target descriptor must be a string")
        end
        local lua_type = type(v)
        if t == "string" or t == "datetime" or t == "path" or t == "url" then
            if lua_type ~= "string" then error("Expected " .. t .. " but got " .. lua_type) end
        elseif t == "integer" or t == "float" or t == "number" then
            if lua_type ~= "number" then error("Expected " .. t .. " but got " .. lua_type) end
        elseif t == "boolean" then
            if lua_type ~= "boolean" then error("Expected boolean but got " .. lua_type) end
        elseif t == "array" or t == "object" or t == "list" then
            if lua_type ~= "table" then error("Expected table but got " .. lua_type) end
        else
            error("Unknown type check target: " .. tostring(t))
        end
        return v
    end,

    has = function(val, needle, opts)
        if val == nil or needle == nil then
            return false
        end

        opts = opts or {}
        local exact = opts.exact == true
        local nocase = opts.nocase == true

        local function matches(candidate)
            if type(candidate) == "string" and type(needle) == "string" then
                local hay, want = candidate, needle
                if nocase then
                    hay, want = hay:lower(), want:lower()
                end
                if exact then
                    return hay == want
                end
                return hay:find(want, 1, true) ~= nil
            end

            return candidate == needle
        end

        if type(val) == "table" then
            for _, candidate in ipairs(val) do
                if matches(candidate) then
                    return true
                end
            end
            return false
        end

        return matches(val)
    end,

    coalesce = function(...)
        local n = select("#", ...)
        for i = 1, n do
            local v = select(i, ...)
            if not is_blank(v) then
                return v
            end
        end
        return nil
    end
}
