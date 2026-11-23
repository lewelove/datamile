-- Runtime plumbing shared by every domain: locate modules, then record every
-- script a config pulls in so hot reloads can watch them.

if not package.searchpath then
    package.searchpath = function(name, path)
        local sep = package.config:sub(1, 1)
        name = name:gsub("%.", sep)
        for c in package.path:gmatch("[^;]+") do
            local filename = c:gsub("%?", name)
            local f = io.open(filename, "r")
            if f then
                f:close()
                return filename
            end
        end
        return nil, "not found"
    end
end

local original_require = require
_G.require = function(modname)
    local path = package.searchpath(modname, package.path)
    if path then
        __DATAMILE_RECORD_DEPENDENCY(path)
    end
    return original_require(modname)
end

local original_dofile = dofile
_G.dofile = function(filename)
    if filename then
        __DATAMILE_RECORD_DEPENDENCY(filename)
    end
    return original_dofile(filename)
end

local original_loadfile = loadfile
_G.loadfile = function(filename, mode, env)
    if filename then
        __DATAMILE_RECORD_DEPENDENCY(filename)
    end
    return original_loadfile(filename, mode, env)
end
