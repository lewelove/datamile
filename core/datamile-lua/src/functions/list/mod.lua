local M = {}

function M.extend(dst, src, start, finish)
    if type(dst) ~= "table" then
        error(string.format("dst: expected table, got %s", type(dst)))
    end
    if type(src) ~= "table" then
        error(string.format("src: expected table, got %s", type(src)))
    end
    local s = start and (start > 1 and start or 1) or 1
    local f = finish or #src
    for i = s, f do
        dst[#dst + 1] = src[i]
    end
    return dst
end

function M.slice(list, start, finish)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end
    local new_list = {}
    local s = start and (start > 1 and start or 1) or 1
    local f = finish or #list
    for i = s, f do
        new_list[#new_list + 1] = list[i]
    end
    return new_list
end

function M.contains(list, value)
    if type(list) ~= "table" then
        return false
    end
    for _, v in ipairs(list) do
        if v == value then
            return true
        end
    end
    return false
end

function M.unique(t, key)
    if type(t) ~= "table" then
        error(string.format("t: expected table, got %s", type(t)))
    end
    local seen = {}
    local finish = #t
    local fn = type(key) == "function" and key
        or (type(key) == "string" and function(x) return type(x) == "table" and x[key] or nil end)
        or function(x) return x end
    local j = 1
    for i = 1, finish do
        local v = t[i]
        if v ~= nil then
            local vh = fn(v)
            if not seen[vh] then
                t[j] = v
                if vh ~= nil then
                    seen[vh] = true
                end
                j = j + 1
            end
        else
            j = i + 1
        end
    end
    for i = j, finish do
        t[i] = nil
    end
    return t
end

function M.bisect(t, val, opts)
    if type(t) ~= "table" then
        error(string.format("expected table, got %s", type(t)))
    end
    opts = opts or {}
    local key = opts.key
    local bound = opts.bound or "lower"
    local low = 1
    local high = #t
    while low <= high do
        local mid = math.floor((low + high) / 2)
        local mid_val = key and t[mid][key] or t[mid]
        local cmp_val = key and (type(val) == "table" and val[key] or val) or val
        local cmp = (bound == "upper" and mid_val <= cmp_val) or (bound ~= "upper" and mid_val < cmp_val)
        if cmp then
            low = mid + 1
        else
            high = mid - 1
        end
    end
    return low
end

return M
