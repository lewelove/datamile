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

local function scalar_cmp(a, b)
    if a == b then
        return 0
    end
    if a == nil then
        return -1
    end
    if b == nil then
        return 1
    end

    local ta, tb = type(a), type(b)
    if ta == "string" and tb == "string" then
        return d.str.natcmp(a, b)
    end
    if ta == tb and (ta == "number" or ta == "boolean") then
        if a < b then
            return -1
        end
        return 1
    end

    local rank = { number = 1, string = 2, boolean = 3 }
    local ra, rb = rank[ta] or 4, rank[tb] or 4
    if ra ~= rb then
        return ra < rb and -1 or 1
    end

    return tostring(a) < tostring(b) and -1 or 1
end

local function key_cmp(a, b)
    if type(a) == "table" and type(b) == "table" then
        local n = math.max(#a, #b)
        for i = 1, n do
            local cmp = scalar_cmp(a[i], b[i])
            if cmp ~= 0 then
                return cmp
            end
        end
        return 0
    end

    return scalar_cmp(a, b)
end

function M.sort_by(list, key_fn)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end
    if type(key_fn) ~= "function" then
        error(string.format("key_fn: expected function, got %s", type(key_fn)))
    end

    table.sort(list, function(a, b)
        return key_cmp(key_fn(a), key_fn(b)) < 0
    end)

    return list
end

function M.sum(list, fn)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end

    local total = 0
    for i, item in ipairs(list) do
        local value = item
        if fn then
            value = fn(item, i)
        end
        if value ~= nil then
            total = total + value
        end
    end

    return total
end

local function extreme(list, fn, better)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end

    local best
    local found = false
    for i, item in ipairs(list) do
        local value = item
        if fn then
            value = fn(item, i)
        end
        if value ~= nil and (not found or better(value, best)) then
            best = value
            found = true
        end
    end

    return found and best or nil
end

function M.min(list, fn)
    return extreme(list, fn, function(a, b) return a < b end)
end

function M.max(list, fn)
    return extreme(list, fn, function(a, b) return a > b end)
end

function M.all(list, pred)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end
    if type(pred) ~= "function" then
        error(string.format("pred: expected function, got %s", type(pred)))
    end

    for i, item in ipairs(list) do
        if not pred(item, i) then
            return false
        end
    end

    return true
end

function M.any(list, pred)
    if type(list) ~= "table" then
        error(string.format("list: expected table, got %s", type(list)))
    end
    if type(pred) ~= "function" then
        error(string.format("pred: expected function, got %s", type(pred)))
    end

    for i, item in ipairs(list) do
        if pred(item, i) then
            return true
        end
    end

    return false
end

return M
