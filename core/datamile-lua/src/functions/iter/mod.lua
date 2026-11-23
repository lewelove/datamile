local M = {}

local Iter = {}
Iter.__index = Iter

function Iter:totable()
    local res = {}
    if self._src_table then
        for _, v in ipairs(self._src_table) do
            table.insert(res, v)
        end
    elseif self._gen then
        while true do
            local v = self._gen()
            if v == nil then break end
            table.insert(res, v)
        end
    end
    return res
end

function Iter:next()
    if self._gen then
        return self._gen()
    end
    return nil
end

local function create_iter(src)
    local obj = {}
    if type(src) == "function" then
        obj._gen = src
    elseif type(src) == "table" then
        local idx = 0
        local len = #src
        obj._src_table = src
        obj._gen = function()
            idx = idx + 1
            if idx <= len then
                return src[idx]
            end
            return nil
        end
    else
        error("expected table or iterator function, got " .. type(src))
    end
    return setmetatable(obj, Iter)
end

return setmetatable(M, {
    __call = function(_, src)
        return create_iter(src)
    end
})
