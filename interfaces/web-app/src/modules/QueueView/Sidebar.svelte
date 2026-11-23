<script lang="ts">
import { player } from "../player.svelte.ts";
import { view } from "../../library/view.svelte.ts";
import { setTab } from "../../navigation.svelte.ts";
import { tick } from "svelte";

let { hasPalette = false }: { hasPalette?: boolean } = $props();

let activeId = $derived(player.currentMediaId);
let isStopped = $derived(player.state === "stop");

async function handleFocus() {
  if (activeId) {
    await view.setFocus({ id: activeId }, true);
    await tick();
    await setTab("home");
  }
}
</script>

<aside class="sidebar-panel">
  <div class="sidebar-top-section">
    <div class="sidebar-group top">
      <button
        type="button"
        class="v-btn-icon v-sidebar-button round"
        disabled={!activeId}
        onclick={handleFocus}
        title="Focus Album"
      >
        <span class="icon">arrow_back</span>
      </button>
      {#if hasPalette}
        <button
          type="button"
          class="v-btn-icon v-sidebar-button"
          class:active={view.isShaderEnabled}
          disabled={isStopped}
          onclick={() => view.toggleShader()}
          title="Toggle Shader"
        >
          <span class="icon">colors</span>
        </button>
      {/if}
    </div>
  </div>
</aside>

<style>
.sidebar-panel {
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  padding: 8px;
  box-sizing: border-box;
  box-shadow: none;
  z-index: 100;
  flex-shrink: 0;
  background-color: transparent;
}

:global(.shader-off) .sidebar-panel {
  background-color: var(--bg-panel);
  box-shadow: var(--panel-shadow);
}

.sidebar-top-section {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
}

.sidebar-group {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
}
</style>
