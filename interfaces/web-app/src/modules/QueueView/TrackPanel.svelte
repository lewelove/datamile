<script lang="ts">
import { player } from "../player.svelte.ts";
import { collection } from "../../library/collection.svelte.ts";
import PanelShell from "./PanelShell.svelte";
import QueueHeader from "./QueueHeader.svelte";
import LyricsView from "./LyricsView.svelte";
import PlayerControls from "./PlayerControls.svelte";

let currentFile = $derived(player.currentFile);
let activeId = $derived(player.currentAlbumId);
let activeTrackIdx = $derived(player.currentTrackIndex);

let fullAlbum = $derived(activeId ? collection.fullAlbumCache[activeId] : null);
let currentTrackFull = $derived(
  activeTrackIdx !== null && fullAlbum?.album.tracks
    ? fullAlbum.album.tracks[activeTrackIdx] ?? null
    : null
);

let title = $derived(
  String(currentTrackFull?.tags?.title ?? "") || player.title || "Unknown Title"
);
let artist = $derived(
  String(currentTrackFull?.tags?.artist ?? "") || player.artist || "Unknown Artist"
);
let duration = $derived(player.duration || 0);

function formatTime(totalSeconds: number) {
  const s = Math.floor(totalSeconds || 0);
  const m = Math.floor(s / 60);
  const rs = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${m}:${pad(rs)}`;
}

let trackDuration = $derived(
  String(currentTrackFull?.objects?.info?.duration_formatted ?? "") ||
    (duration > 0 ? formatTime(duration) : "")
);

let trackFormat = $derived.by(() => {
  const filePath = currentTrackFull?.location?.path || currentFile || "";
  const cleanPath = filePath.split("?")[0].split("#")[0];
  const dotIndex = cleanPath.lastIndexOf(".");
  if (dotIndex !== -1 && dotIndex < cleanPath.length - 1) {
    const ext = cleanPath.slice(dotIndex + 1);
    if (ext.length <= 5) {
      return ext.toUpperCase();
    }
  }
  const enc = currentTrackFull?.objects?.info?.encoding;
  if (enc) {
    if (enc.toLowerCase() === "mpeg") return "MP3";
    return enc.toUpperCase();
  }
  return "";
});
</script>

<PanelShell side="left">
  <div class="track-panel">
    <QueueHeader
      top={title}
      middle={artist}
      bottomLeft={trackFormat}
      bottomRight={trackDuration}
    />
    <LyricsView track={currentTrackFull} file={currentFile} />
    <PlayerControls />
  </div>
</PanelShell>

<style>
.track-panel {
  width: 100%;
  height: 100%;
  background-color: transparent;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
</style>
