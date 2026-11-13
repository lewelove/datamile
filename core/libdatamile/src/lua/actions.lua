_G.datamile = _G.datamile or {}
_G.d = _G.datamile

local cached_launcher = (jit and jit.os == "OSX") and "open" or "xdg-open"
local function get_launcher()
    return cached_launcher
end

_G.datamile.action("open_folder", {
    label = "Open Folder",
    description = "Open album directory in file manager",
    run = function(ctx)
        local launcher = get_launcher()
        for _, album in ipairs(ctx.albums) do
            if album.path and album.path ~= "" then
                d.system({ launcher, album.path }, { detach = true })
            end
        end
    end
})

_G.datamile.action("open_lock", {
    label = "Open Lock",
    description = "Open the entity lockfile in the default viewer",
    run = function(ctx)
        local launcher = get_launcher()
        for _, album in ipairs(ctx.albums) do
            local p = album.lock_path
            if type(p) == "string" and p ~= "" and d.fs.exists(p) then
                d.system({ launcher, p }, { detach = true })
            end
        end
    end
})

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

_G.datamile.action("open_terminal", {
    label = "Open Terminal",
    description = "Open terminal inside album directory",
    run = function(ctx)
        local term = find_terminal()
        local targets = (ctx.albums and #ctx.albums > 0) and ctx.albums or { { path = "." } }
        for _, album in ipairs(targets) do
            d.system({ term }, { cwd = album.path or ".", detach = true })
        end
    end
})

_G.datamile.action("open_config_in_terminal", {
    label = "Open Config in Terminal",
    description = "Open terminal inside Datamile configuration directory",
    run = function(ctx)
        local term = find_terminal()
        local config_dir = REGISTRY.config_dir or d.fs.normalize("~/.config/datamile")
        d.system({ term }, { cwd = config_dir, detach = true })
    end
})
