_G.datamile = _G.datamile or {}
_G.d = _G.datamile

local function launcher()
    if (jit and jit.os == "OSX") then
        return "open"
    end
    return "xdg-open"
end

local cached_terminal = nil
local function find_terminal()
    if cached_terminal then
        return cached_terminal
    end
    local env_term = os.getenv("TERMINAL")
    if env_term and env_term ~= "" then
        cached_terminal = env_term
        return env_term
    end
    local candidates = {
        "ghostty",
        "kitty",
        "foot",
        "alacritty",
        "wezterm",
        "st",
        "gnome-terminal",
        "konsole",
        "xterm"
    }
    local path_env = os.getenv("PATH") or ""
    for _, term in ipairs(candidates) do
        for dir in path_env:gmatch("[^:]+") do
            local bin = d.fs.joinpath(dir, term)
            if d.fs.exists(bin) then
                cached_terminal = term
                return term
            end
        end
    end
    cached_terminal = "xterm"
    return cached_terminal
end

-- Actions receive `ctx.items`: `{ type, id, path, lock }` per resolved entity.
_G.datamile.action("open_config_in_terminal", {
    label = "Open Config in Terminal",
    description = "Open terminal inside Datamile configuration directory",
    run = function(_ctx)
        local term = find_terminal()
        local config_dir = REGISTRY.config_dir or d.fs.normalize("~/.config/datamile")
        d.system({ term }, { cwd = config_dir, detach = true })
    end
})

_G.datamile.action("open_folder", {
    label = "Open Folder",
    description = "Open media directory in file manager",
    run = function(ctx)
        local cmd = launcher()
        for _, item in ipairs(ctx.items or {}) do
            if item.path and item.path ~= "" then
                d.system({ cmd, item.path }, { detach = true })
            end
        end
    end
})

_G.datamile.action("open_lock", {
    label = "Open Lock",
    description = "Open the compiled lock in default viewer",
    run = function(ctx)
        local cmd = launcher()
        for _, item in ipairs(ctx.items or {}) do
            if item.path and item.type then
                local p = string.format("%s/%s.lock.json", item.path, item.type)
                if d.fs.exists(p) then
                    d.system({ cmd, p }, { detach = true })
                end
            end
        end
    end
})

_G.datamile.action("open_terminal", {
    label = "Open Terminal",
    description = "Open terminal inside media directory",
    run = function(ctx)
        local term = find_terminal()
        local items = (ctx.items and #ctx.items > 0) and ctx.items or { { path = "." } }
        for _, item in ipairs(items) do
            d.system({ term }, { cwd = item.path or ".", detach = true })
        end
    end
})
