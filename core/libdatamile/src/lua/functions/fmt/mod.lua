local M = {}

local function total_seconds(ms)
    local value = tonumber(ms) or 0
    if value < 0 then
        value = 0
    end

    return math.floor(value / 1000)
end

-- Compact human duration: 2700000 -> "45m", 5400000 -> "1h 30m".
function M.duration(ms)
    local minutes = math.floor(total_seconds(ms) / 60)
    local hours = math.floor(minutes / 60)
    local rest = minutes % 60

    if hours > 0 then
        if rest > 0 then
            return string.format("%dh %dm", hours, rest)
        end
        return string.format("%dh", hours)
    end

    return string.format("%dm", rest)
end

-- Clock duration: 65000 -> "1:05", 3665000 -> "1:01:05".
function M.clock(ms)
    local seconds = total_seconds(ms)
    local hours = math.floor(seconds / 3600)
    local minutes = math.floor((seconds % 3600) / 60)
    local rest = seconds % 60

    if hours > 0 then
        return string.format("%d:%02d:%02d", hours, minutes, rest)
    end

    return string.format("%d:%02d", minutes, rest)
end

return M
