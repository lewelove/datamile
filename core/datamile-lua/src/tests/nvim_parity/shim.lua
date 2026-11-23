_G.__DATAMILE_TEST_RESULTS = {}

local NIL = setmetatable({}, {
    __is_nil = true,
    __tostring = function() return "null" end,
})
_G.NIL = NIL

local function deep_compare(a, b)
    if a == b then return true end
    if (a == NIL and b == nil) or (a == nil and b == NIL) then return true end
    if type(a) ~= type(b) then return false end
    if type(a) ~= "table" then return false end
    for k, v in pairs(a) do
        if not deep_compare(v, b[k]) then return false end
    end
    for k, _ in pairs(b) do
        if a[k] == nil and b[k] ~= nil then return false end
    end
    return true
end

local function format_val(v)
    if v == NIL then return "vim.NIL" end
    if type(v) == "table" then
        local ok, s = pcall(d.json.encode, v)
        if ok then return s end
    end
    return tostring(v)
end

local describe_stack = {}
local before_each_stack = {}
local after_each_stack = {}
local finally_stack = nil

local testutil = {
    describe = function(name, fn)
        table.insert(describe_stack, name)
        table.insert(before_each_stack, {})
        table.insert(after_each_stack, {})
        local ok, err = pcall(fn)
        table.remove(describe_stack)
        table.remove(before_each_stack)
        table.remove(after_each_stack)
        if not ok then
            table.insert(_G.__DATAMILE_TEST_RESULTS, {
                name = name,
                ok = false,
                err = tostring(err),
            })
        end
    end,

    it = function(name, fn)
        local suite = table.concat(describe_stack, " > ")
        local full_name = (suite ~= "") and (suite .. " > " .. name) or name

        for _, scope in ipairs(before_each_stack) do
            for _, hook in ipairs(scope) do hook() end
        end
        finally_stack = {}

        local ok, err = pcall(fn)

        for _, cleanup in ipairs(finally_stack) do pcall(cleanup) end
        finally_stack = nil

        for i = #after_each_stack, 1, -1 do
            for j = #after_each_stack[i], 1, -1 do
                pcall(after_each_stack[i][j])
            end
        end

        table.insert(_G.__DATAMILE_TEST_RESULTS, {
            name = full_name,
            ok = ok,
            err = ok and nil or tostring(err),
        })
    end,

    before_each = function(fn)
        if #before_each_stack > 0 then
            table.insert(before_each_stack[#before_each_stack], fn)
        end
    end,

    after_each = function(fn)
        if #after_each_stack > 0 then
            table.insert(after_each_stack[#after_each_stack], fn)
        end
    end,

    setup = function(fn) fn() end,
    teardown = function(fn) fn() end,
    finally = function(fn)
        if finally_stack then table.insert(finally_stack, fn) end
    end,

    eq = function(expected, actual, msg)
        if not deep_compare(expected, actual) then
            error(string.format(
                "Equality assertion failed%s\nExpected: %s\nActual:   %s",
                msg and (": " .. tostring(msg)) or "",
                format_val(expected),
                format_val(actual)
            ), 2)
        end
    end,

    eq_paths = function(expected, actual, msg)
        local exp_norm = type(expected) == "string" and d.fs.normalize(expected) or expected
        local act_norm = type(actual) == "string" and d.fs.normalize(actual) or actual
        if not deep_compare(exp_norm, act_norm) then
            error(string.format(
                "Path equality assertion failed%s\nExpected: %s\nActual:   %s",
                msg and (": " .. tostring(msg)) or "",
                format_val(expected),
                format_val(actual)
            ), 2)
        end
    end,

    neq = function(expected, actual, msg)
        if deep_compare(expected, actual) then
            error(string.format(
                "Inequality assertion failed%s\nValue: %s",
                msg and (": " .. tostring(msg)) or "",
                format_val(actual)
            ), 2)
        end
    end,

    ok = function(cond, msg)
        if not cond then
            error(msg or "Assertion failed: condition is not true", 2)
        end
    end,

    matches = function(pat, str)
        local s = tostring(str)
        if not s:match(pat) then
            error(string.format("Pattern match failed:\nPattern: %s\nString:  %s", tostring(pat), s), 2)
        end
    end,

    pcall_err = function(fn, ...)
        local ok, err = pcall(fn, ...)
        if ok then
            error("Expected function to error, but it succeeded", 2)
        end
        return tostring(err)
    end,

    dedent = function(s)
        if type(s) ~= "string" then return s end
        local lines = {}
        local min_indent = nil
        for line in s:gmatch("([^\r\n]*)[\r\n]?") do
            if #line > 0 then
                local indent = line:match("^(%s*)")
                if #line > #indent then
                    if not min_indent or #indent < min_indent then
                        min_indent = #indent
                    end
                end
            end
            table.insert(lines, line)
        end
        if lines[#lines] == "" then table.remove(lines) end
        if lines[1] == "" then table.remove(lines, 1) end
        min_indent = min_indent or 0
        for i, line in ipairs(lines) do
            lines[i] = line:sub(min_indent + 1)
        end
        return table.concat(lines, "\n")
    end,

    is_os = function(name)
        if name == "win" or name == "windows" then
            return false
        end
        return name == "linux" or name == "unix" or name == "posix"
    end,

    paths = {
        test_build_dir = "/tmp/datamile_test_build",
        test_source_path = "/tmp/datamile_test_source",
    },

    mkdir = function(dir) os.execute("mkdir -p " .. dir) end,
    rmdir = function(dir) os.execute("rm -rf " .. dir) end,
    write_file = function(path, content)
        local f = io.open(path, "w")
        if f then
            f:write(content)
            f:close()
        end
    end,
    read_file = function(path)
        local f = io.open(path, "r")
        if f then
            local c = f:read("*a")
            f:close()
            return c
        end
        return nil
    end,
    tmpname = function(is_dir)
        local p = os.tmpname()
        if is_dir then
            os.remove(p)
            os.execute("mkdir -p " .. p)
        end
        return p
    end,
    fix_slashes = function(p) return p and p:gsub("\\", "/") or p end,
}

local function testnvim()
    local n = {}
    n.nvim_dir = testutil.paths.test_build_dir .. "/bin"
    n.nvim_prog = n.nvim_dir .. "/nvim"
    n.nvim_prog_basename = "nvim"

    n.clear = function()
        testutil.mkdir(testutil.paths.test_build_dir)
        testutil.mkdir(n.nvim_dir)
        testutil.write_file(n.nvim_prog, "")
        testutil.mkdir(testutil.paths.test_source_path)
        testutil.mkdir(testutil.paths.test_source_path .. "/test/functional/fixtures")
        testutil.write_file(testutil.paths.test_source_path .. "/CMakePresets.json", "")
        testutil.write_file(testutil.paths.test_source_path .. "/test/functional/fixtures/CMakeLists.txt", "")
        testutil.write_file(testutil.paths.test_source_path .. "/test/functional/fixtures/tty-test.c", "")
    end

    n.rmdir = testutil.rmdir
    n.mkdir_p = testutil.mkdir
    n.mkdir = testutil.mkdir

    n.fn = {
        luaeval = function(code)
            local chunk, err = loadstring("return " .. code)
            if not chunk then error(err, 2) end
            return chunk()
        end,
        fnamemodify = function(path, mod)
            if mod == ":h" then
                return d.fs.dirname(path)
            elseif mod == ":t" then
                return d.fs.basename(path)
            end
            return path
        end,
    }

    n.exec_lua = function(code, ...)
        if type(code) == "function" then
            return code(...)
        elseif type(code) == "string" then
            local chunk, err = loadstring(code)
            if not chunk then error(err, 2) end
            return chunk(...)
        end
    end

    n.eval = function(expr)
        local chunk = loadstring("return " .. expr)
        if chunk then return chunk() end
        return nil
    end

    return n
end

package.preload["test.testutil"] = function() return testutil end
package.preload["test.functional.testnvim"] = function() return testnvim end

_G.jit = _G.jit or jit

_G.vim = {
    NIL = NIL,
    fs = d.fs,
    tbl = d.tbl,
    list = d.list,
    str = d.str,
    json = d.json,
    iter = d.iter,
    system = d.system,
    fn = {
        fnamemodify = function(path, mod)
            if mod == ":h" then
                return d.fs.dirname(path)
            elseif mod == ":t" then
                return d.fs.basename(path)
            end
            return path
        end,
    },
    empty_dict = d.tbl.empty_dict,
    isnil = d.tbl.isnil,
    islist = d.tbl.islist,
    isarray = d.tbl.isarray,
    deepcopy = d.tbl.deepcopy,
    deep_equal = d.tbl.deep_equal,
    copy = d.tbl.copy,
    _copy = d.tbl.copy,
    spairs = d.tbl.spairs,
    startswith = d.str.startswith,
    endswith = d.str.endswith,
    trim = d.str.trim,
    pesc = d.str.pesc,
    stricmp = d.str.stricmp,
    split = d.str.split,
    gsplit = d.str.gsplit,
    str_byteindex = d.str.byteindex,
    str_utfindex = d.str.utfindex,
    str_utf_start = d.str.utf_start,
    str_utf_end = d.str.utf_end,
    str_utf_pos = d.str.utf_pos,
    tbl_extend = d.tbl.extend,
    tbl_deep_extend = d.tbl.deep_extend,
    tbl_contains = d.tbl.contains,
    tbl_filter = d.tbl.filter,
    tbl_map = d.tbl.map,
    tbl_keys = d.tbl.keys,
    tbl_values = d.tbl.values,
    tbl_count = d.tbl.count,
    tbl_isempty = d.tbl.isempty,
    tbl_flatten = d.tbl.flatten,
    tbl_get = d.tbl.get,
    tbl_add_reverse_lookup = d.tbl.add_reverse_lookup,
    list_extend = d.list.extend,
    list_slice = d.list.slice,
    list_contains = d.list.contains,
}
