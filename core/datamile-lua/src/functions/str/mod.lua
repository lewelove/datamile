local M = {}

local function is_continuation(b)
    return b and b >= 128 and b < 192
end

function M.startswith(s, prefix)
    if type(s) ~= "string" then
        error(string.format("s: expected string, got %s", type(s)))
    end
    if type(prefix) ~= "string" then
        error(string.format("prefix: expected string, got %s", type(prefix)))
    end
    return s:sub(1, #prefix) == prefix
end

function M.endswith(s, suffix)
    if type(s) ~= "string" then
        error(string.format("s: expected string, got %s", type(s)))
    end
    if type(suffix) ~= "string" then
        error(string.format("suffix: expected string, got %s", type(suffix)))
    end
    return suffix == "" or s:sub(-#suffix) == suffix
end

function M.trim(s)
    if type(s) ~= "string" then
        error(string.format("s: expected string, got %s", type(s)))
    end
    return s:gsub("^%s+", ""):match("^.*%S") or ""
end

function M.pesc(s)
    if type(s) ~= "string" then
        error(string.format("s: expected string, got %s", type(s)))
    end
    return (s:gsub("[%^%$%(%)%%%.%[%]%*%+%-%?]", "%%%1"))
end

function M.stricmp(a, b)
    if type(a) ~= "string" then
        error(string.format("a: expected string, got %s", type(a)))
    end
    if type(b) ~= "string" then
        error(string.format("b: expected string, got %s", type(b)))
    end
    local la, lb = a:lower(), b:lower()
    if la < lb then
        return -1
    elseif la > lb then
        return 1
    else
        return 0
    end
end

function M.gsplit(s, sep, opts)
    if type(s) ~= "string" then
        error(string.format("s: expected string, got %s", type(s)))
    end
    if type(sep) ~= "string" then
        error(string.format("sep: expected string, got %s", type(sep)))
    end
    local plain = false
    local trimempty = false
    if type(opts) == "boolean" then
        plain = opts
    elseif type(opts) == "table" then
        plain = opts.plain or false
        trimempty = opts.trimempty or false
    elseif opts ~= nil then
        error(string.format("opts: expected table, got %s", type(opts)))
    end

    local start = 1
    local done = false
    local segs = {}
    local empty_start = true

    local function _pass(i, j, ...)
        if i then
            assert(j + 1 > start, "Infinite loop detected")
            local seg = s:sub(start, i - 1)
            start = j + 1
            return seg, ...
        else
            done = true
            return s:sub(start)
        end
    end

    return function()
        if trimempty and #segs > 0 then
            return table.remove(segs)
        elseif done or (s == "" and sep == "") then
            return nil
        elseif sep == "" then
            if start == #s then
                done = true
            end
            return _pass(start + 1, start)
        end

        local seg = _pass(s:find(sep, start, plain))

        if trimempty and seg ~= "" then
            empty_start = false
        elseif trimempty and seg == "" then
            while not done and seg == "" do
                table.insert(segs, 1, "")
                seg = _pass(s:find(sep, start, plain))
            end
            if done and seg == "" then
                return nil
            elseif empty_start then
                empty_start = false
                segs = {}
                return seg
            end
            if seg ~= "" then
                table.insert(segs, 1, seg)
            end
            return table.remove(segs)
        end

        return seg
    end
end

function M.split(s, sep, opts)
    local t = {}
    for c in M.gsplit(s, sep, opts) do
        table.insert(t, c)
    end
    return t
end

function M.word_count(s)
    if type(s) ~= "string" then
        error(string.format("expected string, got %s", type(s)))
    end
    local count = 0
    for _ in s:gmatch("%S+") do
        count = count + 1
    end
    return count
end

function M.utf_pos(s)
    if type(s) ~= "string" then
        error(string.format("expected string, got %s", type(s)))
    end
    local pos = {}
    local len = #s
    local i = 1
    while i <= len do
        table.insert(pos, i)
        local b = s:byte(i)
        if not b then
            break
        elseif b < 0x80 then
            i = i + 1
        elseif b < 0xE0 then
            i = i + 2
        elseif b < 0xF0 then
            i = i + 3
        else
            i = i + 4
        end
    end
    return pos
end

function M.byteindex(s, index)
    if type(s) ~= "string" then
        error(string.format("expected string for s, got %s", type(s)))
    end
    if type(index) ~= "number" then
        error(string.format("expected number for index, got %s", type(index)))
    end
    local pos = M.utf_pos(s)
    if index <= 0 then
        return 1
    elseif index > #pos then
        return #s + 1
    else
        return pos[index]
    end
end

function M.utfindex(s, index)
    if type(s) ~= "string" then
        error(string.format("expected string for s, got %s", type(s)))
    end
    if #s == 0 then
        return 0
    end
    local byte_idx = index or #s
    if byte_idx <= 1 then
        return 1
    end
    local pos = M.utf_pos(s)
    for i = 1, #pos do
        if i == #pos or pos[i + 1] > byte_idx then
            return i
        end
    end
    return #pos
end

function M.utf_start(s, index)
    if type(s) ~= "string" then
        error(string.format("expected string for s, got %s", type(s)))
    end
    if type(index) ~= "number" or index < 1 or index > #s then
        error("index out of range")
    end
    local offset = 0
    while index + offset > 1 and is_continuation(s:byte(index + offset)) do
        offset = offset - 1
    end
    return offset
end

function M.utf_end(s, index)
    if type(s) ~= "string" then
        error(string.format("expected string for s, got %s", type(s)))
    end
    if type(index) ~= "number" or index < 1 or index > #s then
        error("index out of range")
    end
    local start_idx = index + M.utf_start(s, index)
    local b = s:byte(start_idx)
    local char_len = 1
    if b >= 192 and b < 224 then
        char_len = 2
    elseif b >= 224 and b < 240 then
        char_len = 3
    elseif b >= 240 then
        char_len = 4
    end
    local end_idx = math.min(#s, start_idx + char_len - 1)
    return end_idx - index
end

return M
