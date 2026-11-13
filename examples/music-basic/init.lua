d.config({
    storage = {
        music_directory = "~/Music",
        cache = "~/.cache/datamile",
        state = "~/.local/share/datamile",
    },
    compiler = {
        audio_extensions = { "flac", "mp3", "m4a", "ogg", "opus", "wav", "aiff" },
    },
})

require("album")
require("track")
