local M = {}

local function is_absolute(p)
    return p:sub(1, 1) == "/" or p:match("^//") ~= nil or p:match("^%a:[/\\]") ~= nil
end

function M.basename(file)
    if file == nil then return nil end
    if type(file) ~= "string" then
        error(string.format("expected string, got %s", type(file)))
    end
    if file == "" or file == "/" then
        return ""
    end
    if file:sub(-1) == "/" then
        return ""
    end
    return file:match("[^/]+$") or ""
end

function M.dirname(file)
    if file == nil then return nil end
    if type(file) ~= "string" then
        error(string.format("expected string, got %s", type(file)))
    end
    if file == "" then
        return "."
    end
    if file == "/" or file:match("^/+$") then
        return "/"
    end
    if file:sub(-1) == "/" then
        local trimmed = file:gsub("/+$", "")
        return trimmed == "" and "/" or trimmed
    end
    local dir = file:match("^(.*)/[^/]*$")
    if not dir then
        return "."
    end
    dir = dir:gsub("/+$", "")
    if dir == "" then
        return file:sub(1, 1) == "/" and "/" or "."
    end
    return dir
end

function M.normalize(path, opts)
    if path == nil then return nil end
    if type(path) ~= "string" then
        error(string.format("expected string, got %s", type(path)))
    end
    if path == "" then return "" end

    opts = opts or {}
    local expand_env = opts.expand_env ~= false
    local p = path

    if expand_env then
        p = p:gsub("%$([%w_]+)", os.getenv)
        p = p:gsub("%${([%w_]+)}", os.getenv)
        if p:sub(1, 1) == "~" then
            local home = os.getenv("HOME") or ""
            p = home .. p:sub(2)
        end
    end

    local is_unc = p:match("^//[^/]") ~= nil
    p = p:gsub("//+", "/")

    local is_abs = p:sub(1, 1) == "/"
    local parts = {}
    for part in p:gmatch("[^/]+") do
        if part == ".." then
            if #parts > 0 and parts[#parts] ~= ".." then
                table.remove(parts)
            elseif not is_abs then
                table.insert(parts, "..")
            end
        elseif part ~= "." then
            table.insert(parts, part)
        end
    end

    local prefix = is_unc and "//" or (is_abs and "/" or "")
    local res = prefix .. table.concat(parts, "/")
    if res == "" then
        return is_abs and "/" or "."
    end
    return res
end

function M.joinpath(...)
    local n = select("#", ...)
    local parts = {}
    for i = 1, n do
        local item = select(i, ...)
        if item ~= nil and item ~= "" then
            table.insert(parts, tostring(item))
        end
    end
    if #parts == 0 then return "" end
    local path = table.concat(parts, "/")
    local is_unc = path:match("^//[^/]") ~= nil
    local clean = path:gsub("//+", "/")
    if is_unc then
        return "/" .. clean
    end
    return clean
end

function M.abspath(path, opts)
    if path == nil then return nil end
    if type(path) ~= "string" then
        error(string.format("expected string, got %s", type(path)))
    end
    opts = opts or {}
    local cwd = opts.cwd and M.normalize(opts.cwd, { expand_env = opts.plain ~= true }) or os.getenv("PWD") or "."
    if path == "." or path == "" then
        return cwd
    end
    local plain = opts.plain or false
    local p = path
    if not plain and p:sub(1, 1) == "~" then
        local home = os.getenv("HOME") or "/root"
        p = home .. p:sub(2)
    end
    if is_absolute(p) then
        return p
    end
    return cwd .. "/" .. p
end

function M.relpath(base, target)
    if type(base) ~= "string" or type(target) ~= "string" then
        return nil
    end
    local is_base_abs = is_absolute(base)
    local is_target_abs = is_absolute(target)
    local b_path = base
    local t_path = target
    if is_base_abs ~= is_target_abs then
        if not is_base_abs then
            b_path = M.abspath(base)
        end
        if not is_target_abs then
            t_path = M.abspath(target)
        end
    end
    local b = M.normalize(b_path)
    local t = M.normalize(t_path)
    if b == t then
        return "."
    end
    if b == "/" then
        if t:sub(1, 1) == "/" then
            return t:sub(2)
        end
        return nil
    end
    local prefix = b .. "/"
    if t:sub(1, #prefix) == prefix then
        return t:sub(#prefix + 1)
    end
    return nil
end

function M.parents(start)
    if type(start) ~= "string" then
        error(string.format("expected string, got %s", type(start)))
    end
    return function(_, dir)
        local parent = M.dirname(dir)
        if parent and parent ~= dir and parent ~= "." then
            return parent
        end
        return nil
    end, nil, start
end

function M.dir(path, opts)
    opts = opts or {}
    local norm = M.normalize(path)
    local depth = opts.depth or 1
    local skip = opts.skip
    local follow = opts.follow or false

    if depth == 1 then
        return M._scandir_native(norm, follow)
    end

    local queue = { { path = norm, rel = "", depth = 1 } }
    local queue_idx = 1
    local cur_iter = nil
    local cur_node = nil

    return function()
        while true do
            if not cur_iter then
                if queue_idx > #queue then
                    return nil
                end
                cur_node = queue[queue_idx]
                queue_idx = queue_idx + 1
                cur_iter = M._scandir_native(cur_node.path, follow)
            end

            local name, type_ = cur_iter()
            if name then
                local rel = (cur_node.rel == "") and name or (cur_node.rel .. "/" .. name)
                if type_ == "directory" and cur_node.depth < depth then
                    local should_traverse = true
                    local skip_full = cur_node.path .. "/" .. name
                    if skip and skip(rel, skip_full) == false then
                        should_traverse = false
                    end
                    if should_traverse then
                        table.insert(queue, {
                            path = cur_node.path .. "/" .. name,
                            rel = rel,
                            depth = cur_node.depth + 1,
                        })
                    end
                end
                return rel, type_
            else
                cur_iter = nil
            end
        end
    end
end

function M.find(names, opts)
    opts = opts or {}
    if type(names) ~= "string" and type(names) ~= "table" and type(names) ~= "function" then
        error(string.format("expected string, table, or function for names, got %s", type(names)))
    end

    local start = opts.path and M.normalize(opts.path) or "."
    local upward = opts.upward or false
    local stop = opts.stop and M.normalize(opts.stop) or nil
    local limit = opts.limit or (upward and 1 or math.huge)
    local match_type = opts.type

    local matches = {}
    local function make_matcher(target)
        if type(target) == "function" then
            return target
        elseif type(target) == "table" then
            local set = {}
            for _, n in ipairs(target) do
                set[n] = true
            end
            return function(name)
                return set[name] == true
            end
        else
            return function(name)
                return name == target
            end
        end
    end

    local match_fn = make_matcher(names)

    if upward then
        local cur = start
        while cur and #matches < limit do
            if stop and cur == stop then
                break
            end
            for name, type_ in M.dir(cur, { depth = 1 }) do
                local type_ok = (match_type == nil) or (match_type == type_)
                local full = M.joinpath(cur, name)
                if type_ok and match_fn(name, full) then
                    table.insert(matches, full)
                    if #matches >= limit then
                        return matches
                    end
                end
            end
            local parent = M.dirname(cur)
            if not parent or parent == cur or parent == "." then
                break
            end
            cur = parent
        end
    else
        for rel, type_ in M.dir(start, { depth = opts.depth or math.huge, skip = opts.skip }) do
            local full = M.joinpath(start, rel)
            if stop and (full == stop or rel == stop) then
                break
            end
            local type_ok = (match_type == nil) or (match_type == type_)
            local bname = M.basename(rel)
            if type_ok and match_fn(bname, full) then
                table.insert(matches, full)
                if #matches >= limit then
                    break
                end
            end
        end
    end

    return matches
end

function M.first(dir, candidates)
    if type(dir) ~= "string" then
        error(string.format("dir: expected string, got %s", type(dir)))
    end
    if type(candidates) == "string" then
        candidates = { candidates }
    end
    if type(candidates) ~= "table" then
        error(string.format("candidates: expected table, got %s", type(candidates)))
    end

    for _, name in ipairs(candidates) do
        local candidate = M.joinpath(dir, name)
        if M.exists(candidate) then
            return candidate
        end
    end

    -- Missing candidates already record the directory; an empty list does not.
    if #candidates == 0 then
        M.exists(dir)
    end

    return nil
end

function M.files(dir, opts)
    opts = opts or {}
    if type(dir) ~= "string" then
        error(string.format("dir: expected string, got %s", type(dir)))
    end

    local exts = opts.ext
    if type(exts) == "string" then
        exts = { exts }
    elseif exts ~= nil and type(exts) ~= "table" then
        error(string.format("ext: expected string or table, got %s", type(exts)))
    end

    local accepted = {}
    for _, ext in ipairs(exts or {}) do
        accepted[ext:lower():gsub("^%.*", "")] = true
    end

    local result = {}
    local depth = opts.depth or math.huge
    for rel, kind in M.dir(dir, { depth = depth, follow = opts.follow or false }) do
        if kind == "file" then
            local full = M.joinpath(dir, rel)
            local ext = M.ext(full)
            if exts == nil or accepted[ext] then
                table.insert(result, full)
            end
        end
    end

    local sort = opts.sort
    if sort == nil then
        sort = "natural"
    end
    if sort == false then
        return result
    end
    if sort == "natural" then
        table.sort(result, function(a, b)
            return d.str.natcmp(a, b) < 0
        end)
        return result
    end
    if sort == "lex" then
        table.sort(result)
        return result
    end

    error(string.format("sort: expected 'natural', 'lex', or false, got %s", tostring(sort)))
end

local function dir_entries(dir_path)
    local entries = {}
    for name, kind in M.dir(dir_path, { depth = 1 }) do
        table.insert(entries, {
            name = name,
            path = M.joinpath(dir_path, name),
            type = kind,
        })
    end

    return entries
end

local function marker_matcher(markers)
    local wanted = {}
    for _, marker in ipairs(markers) do
        wanted[marker] = true
    end

    return function(_, entries)
        for _, entry in ipairs(entries) do
            if wanted[entry.name] then
                return true
            end
        end
        return false
    end
end

function M.roots(scope, matcher_or_opts)
    if type(scope) ~= "table" or type(scope.path) ~= "string" then
        error("scope must be a table with a string path")
    end

    local opts = {}
    local matcher
    if type(matcher_or_opts) == "string" then
        matcher = marker_matcher({ matcher_or_opts })
        opts.prune = true
    elseif type(matcher_or_opts) == "table" then
        local markers = matcher_or_opts.marker
        matcher = matcher_or_opts.match
        if type(markers) == "string" then
            markers = { markers }
        end
        if type(markers) == "table" and matcher == nil then
            matcher = marker_matcher(markers)
            opts.prune = matcher_or_opts.prune ~= false
        else
            opts.prune = matcher_or_opts.prune == true
        end
    elseif type(matcher_or_opts) == "function" then
        matcher = matcher_or_opts
    else
        error("matcher must be a marker string, table, or function")
    end

    if type(matcher) ~= "function" then
        error("matcher must provide a marker or match function")
    end

    local recursive = scope.recursive ~= false
    local selected = {}

    local function evaluate(dir_path)
        local entries = dir_entries(dir_path)
        local first, second = matcher(dir_path, entries)
        if not first then
            return false, false
        end

        local seed = { path = dir_path }
        if type(first) == "table" then
            seed = d.tbl.extend("force", seed, first)
        end
        table.insert(selected, seed)

        local prune = opts.prune == true or second == "prune"
        return true, prune
    end

    local _, root_pruned = evaluate(scope.path)
    if not recursive or root_pruned then
        table.sort(selected, function(a, b)
            return d.str.natcmp(a.path, b.path) < 0
        end)
        return selected
    end

    for _, kind in M.dir(scope.path, {
        depth = math.huge,
        skip = function(_, full)
            local _, prune = evaluate(full)
            return not prune
        end,
    }) do
        if kind == nil then
            break
        end
    end

    table.sort(selected, function(a, b)
        return d.str.natcmp(a.path, b.path) < 0
    end)
    return selected
end

function M.root(source, marker)
    if source == nil or marker == nil then
        error("missing required arguments: source and marker")
    end
    local start = type(source) == "string" and source or "."
    start = M.normalize(start)

    local marker_list = type(marker) == "table" and marker or { marker }
    for _, item in ipairs(marker_list) do
        local res = M.find(item, {
            path = start,
            upward = true,
            limit = 1,
        })
        if #res > 0 then
            return M.dirname(res[1])
        end
    end
    return nil
end

return M
