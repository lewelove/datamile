<script lang="ts">
import { fade, type FadeParams } from "svelte/transition";
import { cubicOut } from "svelte/easing";
import { view } from "../../../library/view.svelte.ts";
import { playAlbum, playDisc, executeAction, updateAlbum } from "../../../api.ts";
import ClearCover from "../../ClearCover.svelte";
import ModalDrawerTracks from "./ModalDrawerTracks.svelte";
import type { FullAlbum } from "../../../types.ts";

let { album, onclose }: { album: FullAlbum; onclose: () => void } = $props();

let windowWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1280);
let leftColumnWidth = $derived(Math.round(0.36 * windowWidth));

let albumData = $derived(album.album);
let infoData = $derived(albumData.objects?.info || {});

let coverHash = $derived(albumData.images?.main?.[0]?.address || "");
let title = $derived(String(albumData.tags?.album ?? "") || "Untitled");
let artist = $derived(String(albumData.tags?.albumartist ?? "") || "Unknown");
let dateString = $derived(String(albumData.tags?.date ?? ""));

let discCount = $derived(Number(infoData.total_discs) || 1);
let trackCount = $derived(Number(infoData.total_tracks) || 0);
let durationStr = $derived(infoData.duration_formatted || "--:--");

let isReady = $state(false);

$effect(() => {
  if (!coverHash) {
    isReady = true;
    return;
  }

  const dpr = window.devicePixelRatio || 1;
  const targetWidth = Math.round((leftColumnWidth - 64) * dpr);
  const srcUrl = `/api/images/${coverHash}.bmp?size=${targetWidth}&filter=catmullrom`;

  const img = new Image();
  img.src = srcUrl;
  img.decode().finally(() => {
    isReady = true;
  });
});

function fadeIf(node: Element, params: FadeParams) {
  if (view.isFocusInstant) {
    return { duration: 0, css: () => "" };
  }
  return fade(node, params);
}

async function handlePlay() {
  try {
    await playAlbum(albumData.id);
  } catch {}
}

async function handleUpdate() {
  try {
    await updateAlbum(albumData.id);
  } catch {}
}

async function handlePlayTrack(index: number) {
  try {
    const tracks = albumData.tracks || [];
    const track = tracks[index];
    if (!track) {
      return;
    }
    const discNumber = Number(track.tags?.discnumber ?? 1);

    let intraDiscOffset = 0;
    for (let i = 0; i < index; i++) {
      if (Number(tracks[i].tags?.discnumber ?? 1) === discNumber) {
        intraDiscOffset++;
      }
    }

    await playDisc(albumData.id, discNumber, intraDiscOffset);
  } catch {}
}

async function handlePlayDisc(discNumber: number) {
  try {
    await playDisc(albumData.id, discNumber);
  } catch {}
}

function handleBackdropClick(e: MouseEvent) {
  if (e.target === e.currentTarget) {
    onclose();
  }
}
</script>

<svelte:window bind:innerWidth={windowWidth} />

{#if isReady}
<div
  class="modal-backdrop"
  onclick={handleBackdropClick}
  role="presentation"
  in:fadeIf={{ duration: 200, easing: cubicOut }}
  out:fade={{ duration: 50, easing: cubicOut }}
>
  <div class="modal-chassis v-panel">
    <div class="modal-content">
      <div class="column-left">
        <div class="cover-container" style:height="{leftColumnWidth - 64}px">
          <ClearCover
            hash={coverHash}
            width={leftColumnWidth - 64}
            height={leftColumnWidth - 64}
          />
        </div>

        <div class="meta-container">
          <h2 class="album-title">{title}</h2>
          <h3 class="album-artist">{artist}</h3>

          {#if dateString}
            <span class="v-mono meta-date">{dateString}</span>
          {/if}

          <div class="meta-stack">
            <div class="v-mono meta-row">
              <span class="v-truncate meta-val">{durationStr}</span>

              {#if discCount > 1}
                <span class="meta-sep">•</span>
                <span class="v-truncate meta-val">{discCount} Discs</span>
              {/if}

              <span class="meta-sep">•</span>
              <span class="v-truncate meta-val">{trackCount} Tracks</span>
            </div>
          </div>
        </div>
      </div>

      <div class="column-right">
        <div class="button-bar">
          <div class="bar-group">
            <button type="button" class="v-btn-icon icon-btn" onclick={handleUpdate} title="Update Album">
              <span class="icon">refresh</span>
            </button>
            <button
              type="button"
              class="v-btn-icon icon-btn"
              onclick={() => executeAction("open_folder", albumData.id)}
              title="Open Local Folder"
            >
              <span class="icon">folder</span>
            </button>
            <button
              type="button"
              class="v-btn-icon icon-btn"
              onclick={() => executeAction("open_manifest", albumData.id)}
              title="Open Manifest"
            >
              <span class="icon">edit_document</span>
            </button>
            <button
              type="button"
              class="v-btn-icon icon-btn"
              onclick={() => executeAction("open_lock", albumData.id)}
              title="Open Data Object"
            >
              <span class="icon">code</span>
            </button>
            <button
              type="button"
              class="v-btn-icon icon-btn"
              onclick={() => executeAction("open_terminal", albumData.id)}
              title="Open Terminal"
            >
              <span class="icon">terminal_2</span>
            </button>
          </div>

          <div class="bar-group right">
            <button type="button" class="v-btn-icon icon-btn" onclick={handlePlay} title="Play Album">
              <span class="icon filled">play_arrow</span>
            </button>
          </div>
        </div>
        <div class="tracks-scroll-area">
          <ModalDrawerTracks
            tracks={albumData.tracks || []}
            totalDiscs={infoData.total_discs}
            albumArtist={artist}
            onplay={handlePlayTrack}
            onplaydisc={handlePlayDisc}
          />
        </div>
      </div>
    </div>
  </div>
</div>
{/if}

<style>
.button-bar {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 8px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--border-muted);
  width: 100%;
}

.bar-group {
  display: flex;
  align-items: center;
  gap: 8px;
}

.bar-group.right {
  margin-left: auto;
}

.modal-backdrop {
  position: fixed;
  inset: 0;
  background-color: var(--bg-modal-backdrop);
  backdrop-filter: blur(2px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.modal-chassis {
  height: 88vh;
  width: 80vw;
}

.modal-content {
  display: grid;
  grid-template-columns: 45% 55%;
  grid-template-rows: 100%;
  height: 100%;
  width: 100%;
  min-height: 0;
}

.column-left {
  display: flex;
  flex-direction: column;
  padding: 32px;
  background-color: var(--bg-modal-left);
  min-width: 0;
  min-height: 0;
  box-sizing: border-box;
}

.cover-container {
  width: 100%;
  flex-shrink: 0;
  background-color: transparent;
  overflow: visible;
}

.meta-container {
  margin-top: 16px;
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}

.album-title {
  margin: 0;
  font-size: 25px;
  font-weight: 400;
  color: var(--text-main);
  overflow-wrap: break-word;
}

.album-artist {
  margin: 5px 0 0;
  font-size: 20px;
  font-weight: 400;
  color: var(--text-muted);
  overflow-wrap: break-word;
}

.meta-stack {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.meta-row,
.meta-date {
  display: flex;
  align-items: center;
  font-size: 16px;
  color: var(--text-muted);
  gap: 12px;
}

.meta-date {
  margin: 12px 0 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.meta-sep {
  color: var(--text-subtle);
  font-size: 12px;
  flex-shrink: 0;
}

.column-right {
  display: flex;
  flex-direction: column;
  padding: 32px;
  min-width: 0;
  min-height: 0;
  height: 100%;
  box-sizing: border-box;
  background-color: var(--bg-modal-right);
}

.icon-btn {
  width: 32px;
  height: 32px;
  border-radius: 16px;
}

.tracks-scroll-area {
  position: relative;
  flex: 1;
  overflow-y: scroll;
  min-height: 0;
  background-color: var(--bg-panel);
  transform: translateZ(0);
  border-bottom: 1px solid var(--border-muted);
  padding: 12px 0;
  scrollbar-width: none;
  mask-image: linear-gradient(
    to bottom,
    transparent 0,
    black 12px,
    black calc(100% - 12px),
    transparent 100%
  );
}

.tracks-scroll-area::-webkit-scrollbar {
  width: 0;
}
</style>
