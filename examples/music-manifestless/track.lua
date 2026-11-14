local util = require("util")

local track = d.media("track", {
  kind = "file",
  select = function(parent)
    return util.tracks(parent.path)
  end,
})

track.input("audio", function(ctx)
  if ctx.path == nil then
    return nil
  end

  return d.fs.read_audio(ctx.path)
end)

track.id(function(ctx)
  local tags = ctx.tags
  local disc = tonumber(tags.discnumber or 1)
  local number = tostring(tags.tracknumber or "")
  if disc > 1 then
    number = tostring(tags.discnumber) .. "." .. number
  end

  return table.concat({
    tags.albumartist or tags.artist or "",
    tags.album or "",
    number,
    tags.title or "",
  }, " - ")
end)

for _, key in ipairs({
  "artist",
  "albumartist",
  "album",
  "title",
  "tracknumber",
  "discnumber",
  "date",
  "genre",
  "composer",
}) do
  track.tag(key, function(ctx)
    return d.get(ctx, "manifest." .. key) or d.get(ctx, "audio.tags." .. key)
  end)
end

track.object("info", function(ctx)
  local audio = ctx.audio
  if not audio then
    return nil
  end

  return {
    sample_rate = audio.sample_rate,
    bits_per_sample = audio.bit_depth or 0,
    bitrate_kbps = audio.audio_bitrate or 0,
    encoding = audio.format,
    channels = audio.channels or 0,
    duration_milliseconds = audio.duration_milliseconds or 0,
    duration_formatted = d.fmt.clock(audio.duration_milliseconds or 0),
  }
end)
