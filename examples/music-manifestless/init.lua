d.config({
    storage = {
        cache = "~/.cache/datamile",
        state = "~/.local/share/datamile",
    },
    compiler = {
        audio_extensions = { "flac", "mp3", "m4a", "ogg", "opus", "wav", "aiff" },
    },
})

require("album")
require("track")
