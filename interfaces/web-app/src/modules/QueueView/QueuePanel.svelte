<script lang="ts">
import { player } from "../player.svelte.ts";
import { collection } from "../../library/collection.svelte.ts";
import PanelShell from "./PanelShell.svelte";
import QueueHeader from "./QueueHeader.svelte";
import TracklistView from "./TracklistView.svelte";
import type { MappedQueueTrack, QueueGroup } from "../../types.ts";

let mappedTracks = $derived(
  player.queue.map((item): MappedQueueTrack => {
    const fullAlbum = item.album_id ? collection.fullAlbumCache[item.album_id] : null;
    const meta =
      item.track_index !== null && item.track_index !== undefined && fullAlbum?.album.tracks
        ? fullAlbum.album.tracks[item.track_index] ?? null
        : null;

    return {
      id: item.id,
      file: item.file,
      isPlaying: player.currentFile === item.file,
      trackNo: meta ? String(meta.tags?.tracknumber ?? "") : item.track_no ?? "#",
      discNo: meta ? Number(meta.tags?.discnumber ?? 1) : item.disc_no ?? 1,
      duration: meta
        ? String(meta.objects?.info?.duration_formatted ?? "")
        : item.duration ?? "",
      durationMs: meta
        ? Number(meta.objects?.info?.duration_milliseconds ?? 0)
        : item.duration_ms ?? 0,
      title: meta ? String(meta.tags?.title ?? "") : item.title || item.file,
      artist: meta ? String(meta.tags?.artist ?? "") : item.artist || "",
      albumId: item.album_id || null
    };
  })
);

let groupedQueue = $derived.by(() => {
  const groups: QueueGroup[] = [];
  mappedTracks.forEach((track) => {
    if (groups.length === 0 || groups[groups.length - 1].albumId !== track.albumId) {
      const albumMeta = track.albumId ? collection.dict[track.albumId] ?? null : null;
      groups.push({
        albumId: track.albumId,
        albumMeta,
        tracks: [track]
      });
    } else {
      groups[groups.length - 1].tracks.push(track);
    }
  });
  return groups;
});
</script>

<PanelShell side="right">
  <div class="queue-panel">
    <div class="queue-scroll">
      {#each groupedQueue as group, groupIdx (group.albumId || groupIdx)}
        {#if group.albumMeta}
          <QueueHeader
            top={String(group.albumMeta.tags?.album ?? "")}
            middle={String(group.albumMeta.tags?.albumartist ?? "")}
            bottomLeft={String(group.albumMeta.tags?.date ?? "").substring(0, 4)}
            bottomRight={String(group.albumMeta.objects?.info?.duration_formatted ?? "")}
          />
        {/if}
        <TracklistView tracks={group.tracks} albumMeta={group.albumMeta} />
      {/each}
    </div>
  </div>
</PanelShell>

<style>
.queue-panel {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  box-sizing: border-box;
  background-color: transparent;
  min-height: 0;
  overflow: hidden;
}

.queue-scroll {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
}

.queue-scroll::-webkit-scrollbar {
  width: 0;
}
</style>
