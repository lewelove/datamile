local M = {}

-- Milliseconds as `M:SS`, or `H:MM:SS` once the hour is non-zero.
function M.duration(ms)
    if type(ms) ~= "number" then
        error(string.format("ms: expected number, got %s", type(ms)))
    end

    local total_seconds = math.floor(ms / 1000)
    local seconds = total_seconds % 60
    local minutes = math.floor(total_seconds / 60) % 60
    local hours = math.floor(total_seconds / 3600)

    if hours > 0 then
        return string.format("%d:%02d:%02d", hours, minutes, seconds)
    end
    return string.format("%d:%02d", minutes, seconds)
end

-- Byte count as a short human-readable string, e.g. `1.5 MB`.
function M.bytes(n)
    if type(n) ~= "number" then
        error(string.format("n: expected number, got %s", type(n)))
    end

    local units = { "B", "KB", "MB", "GB", "TB" }
    local value = n
    local unit = 1
    while value >= 1024 and unit < #units do
        value = value / 1024
        unit = unit + 1
    end

    if unit == 1 then
        return string.format("%d %s", value, units[unit])
    end
    return string.format("%.1f %s", value, units[unit])
end

return M
