<script lang="ts">
let {
  hash,
  width,
  height
}: { hash?: string; width: number; height: number } = $props();

let dpr = $derived(window.devicePixelRatio || 1);
let targetWidth = $derived(Math.round(width * dpr));

let algo = "catmullrom";
let srcUrl = $derived(
  hash && targetWidth > 0 ? `/api/images/${hash}.bmp?size=${targetWidth}&filter=${algo}` : ""
);
</script>

<div class="clear-cover-wrapper" style:width="{width}px" style:height="{height}px">
  {#if hash && srcUrl}
    {#key hash}
      <img src={srcUrl} class="cover-image" alt="" draggable="false" />
    {/key}
  {:else}
    <div class="empty-cover"></div>
  {/if}
</div>

<style>
.clear-cover-wrapper {
  position: relative;
  overflow: visible;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: var(--album-cover-shadow);
}

.cover-image {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.empty-cover {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  box-sizing: border-box;
}
</style>
