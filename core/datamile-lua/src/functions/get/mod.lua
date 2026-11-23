return function(tbl, ...)
    if type(tbl) ~= "table" then
        return nil
    end

    local curr = tbl
    local n = select("#", ...)

    for idx = 1, n do
        if type(curr) ~= "table" then
            return nil
        end

        local arg = select(idx, ...)
        if type(arg) == "string" and arg:find(".", 1, true) then
            for part in arg:gmatch("[^%.]+") do
                if type(curr) ~= "table" then
                    return nil
                end
                local num = tonumber(part)
                if num and math.floor(num) == num then
                    if curr[num] ~= nil then
                        curr = curr[num]
                    else
                        curr = curr[part]
                    end
                else
                    curr = curr[part]
                end
            end
        elseif arg ~= nil then
            curr = curr[arg]
        else
            return nil
        end
    end

    return curr
end
