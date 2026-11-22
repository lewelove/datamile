<script lang="ts">
import { player } from "../player.svelte.ts";
import ClearCover from "../ClearCover.svelte";

let { imageHash = "", width = 0 }: { imageHash?: string; width?: number } = $props();

let persistentHash = $state("");

$effect(() => {
  if (player.currentFile) {
    persistentHash = imageHash;
  }
});
</script>

<div class="cover-wrapper">
  <div class="cover-panel">
    <div class="cover-absolute-wrapper">
      {#if width > 0}
        <ClearCover hash={persistentHash} {width} height={width} />
      {/if}
    </div>
  </div>
</div>

<style>
.cover-wrapper {
  height: 100%;
  max-height: 100%;
  max-width: 60vw;
  display: flex;
  justify-content: center;
  align-items: center;
  position: relative;
  z-index: 10;
  transition: background-color 0.3s ease;
  box-sizing: border-box;
  padding: 20px 0;
  box-shadow: none;
}

.cover-panel {
  height: 100%;
  max-width: 100%;
  aspect-ratio: 1 / 1;
  position: relative;
  box-sizing: border-box;
  background: none;
  padding: 0;
}

.cover-absolute-wrapper {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
