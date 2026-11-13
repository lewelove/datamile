<script lang="ts">
import { jumpToQueueIndex } from "../../api.ts";
import type { AlbumDictEntry, MappedQueueTrack } from "../../types.ts";

let {
  tracks = [],
  albumMeta = null
}: {
  tracks?: MappedQueueTrack[];
  albumMeta?: AlbumDictEntry | null;
} = $props();

function formatDuration(str: string) {
  if (!str) {
    return "0:00";
  }
  const parts = str.split(":");
  while (parts.length > 2 && parseInt(parts[0]) === 0) {
    parts.shift();
  }
  if (parts[0].length > 1 && parts[0].startsWith("0")) {
    parts[0] = parts[0].substring(1);
  }
  return parts.join(":");
}

function formatMs(ms: number) {
  if (!ms) {
    return "0:00";
  }
  const totalSeconds = Math.floor(ms / 1000);
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  const pad = (num: number) => String(num).padStart(2, "0");
  if (h > 0) {
    return `${h}:${pad(m)}:${pad(s)}`;
  }
  return `${m}:${pad(s)}`;
}

function getDiscDuration(discTracks: MappedQueueTrack[], discNumber: number) {
  const totalMs = discTracks
    .filter((t) => t.discNo === discNumber)
    .reduce((acc, t) => acc + t.durationMs, 0);
  return formatMs(totalMs);
}

async function handleJump(id: string | number) {
  try {
    await jumpToQueueIndex(id);
  } catch {}
}

let albumArtist = $derived(String(albumMeta?.tags?.albumartist ?? ""));

let isMultiDiscAlbum = $derived(
  albumMeta && Number(albumMeta.objects?.info?.total_discs ?? 1) > 1
);
</script>

<div class="tracklist-container">
  {#each tracks as track, i (track.id)}
    {@const showDiscHeader =
      isMultiDiscAlbum && (i === 0 || track.discNo !== tracks[i - 1].discNo)}

    {#if showDiscHeader}
      <div class="disc-header-row" class:first-disc={i === 0}>
        <span class="disc-label">Disc {track.discNo}</span>
        <div class="disc-header-right">
          <span class="v-mono disc-duration-label"
            >{getDiscDuration(tracks, track.discNo)}</span
          >
        </div>
      </div>
    {/if}

    <div
      class="v-track-row track-row"
      class:active={track.isPlaying}
      ondblclick={() => handleJump(track.id)}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          handleJump(track.id);
        }
      }}
      role="button"
      tabindex="0"
    >
      <div class="v-track-body">
        <span class="v-truncate v-track-title track-title">{track.title}</span>
        {#if track.artist && albumArtist && track.artist.toLowerCase() !== albumArtist.toLowerCase()}
          <span class="v-truncate v-track-artist track-artist">{track.artist}</span>
        {/if}
      </div>
      <span class="v-mono v-track-meta track-meta">
        {formatDuration(track.duration)}
      </span>
    </div>
  {/each}
</div>

<style>
.tracklist-container {
  width: 100%;
  display: flex;
  flex-direction: column;
}

.disc-header-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0;
  margin: 12px 0 8px;
}

.disc-header-row.first-disc {
  margin-top: 0;
}

.disc-header-right {
  display: flex;
  align-items: center;
}

.disc-label {
  display: flex;
  align-items: center;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 500;
  border: 1px solid var(--border-muted);
  border-radius: 8px;
  height: 24px;
  box-sizing: border-box;
  color: var(--text-muted);
}

.disc-duration-label {
  display: flex;
  align-items: center;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 400;
  border: 1px solid var(--border-muted);
  border-radius: 8px;
  height: 24px;
  box-sizing: border-box;
  color: var(--text-muted);
}

.track-row {
  margin: 0;
  padding-left: 14px;
  transition: none !important;
}

.track-row + .track-row {
  margin-top: 4px;
}

.track-title {
  color: var(--text-main);
}

.track-artist {
  color: var(--text-muted);
}

.track-meta {
  color: var(--text-subtle);
}
</style>
