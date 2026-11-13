<script lang="ts">
import type { TrackItem } from "../../types.ts";

let {
  track = null,
  file = null
}: {
  track?: TrackItem | null;
  file?: string | null;
} = $props();

let scrollEl: HTMLDivElement | undefined = $state();

let isInstrumental = $derived(track?.tags?.instrumental === true);

let lyricsText = $derived.by(() => {
  if (!track || track.tags?.instrumental === true) {
    return "";
  }
  if (track.objects?.lyrics?.text) {
    return track.objects.lyrics.text;
  }
  if (typeof track.tags?.lyrics === "string") {
    return track.tags.lyrics;
  }
  return "";
});

$effect(() => {
  void file;
  if (scrollEl) {
    scrollEl.scrollTop = 0;
  }
});
</script>

<div bind:this={scrollEl} class="lyrics-scroll">
  {#key file}
    {#if isInstrumental}
      <div class="instrumental-msg">INSTRUMENTAL</div>
    {:else if lyricsText}
      <div class="lyrics-content">
        {#each lyricsText.split(/\r?\n/) as line, i (i)}
          <p class="lyric-line">{line}</p>
        {/each}
      </div>
    {/if}
  {/key}
</div>

<style>
.lyrics-scroll {
  flex: 1;
  overflow-y: auto;
  width: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  scrollbar-width: none;
  mask-image: linear-gradient(
    to bottom,
    transparent 0,
    black 16px,
    black calc(100% - 16px),
    transparent 100%
  );
}

.lyrics-scroll::-webkit-scrollbar {
  width: 0;
}

.lyrics-content {
  font-family: var(--font-stack);
  font-size: 15px;
  line-height: 1.05;
  color: var(--text-main);
  text-align: center;
  padding: 16px 0;
  width: 100%;
}

.lyric-line {
  margin: 6px 14px;
  min-height: 0.3em;
  text-wrap: balance;
}

.instrumental-msg {
  margin: auto;
  font-family: var(--font-mono);
  font-size: 15px;
  line-height: 1.2;
  color: var(--text-main);
  text-align: center;
  display: flex;
  justify-content: center;
  align-items: center;
  height: 100%;
}
</style>
