<script lang="ts">
import { nav, setTab } from "../navigation.svelte.ts";
import { executeAction } from "../api.ts";

let { variant = "solid" }: { variant?: "solid" | "transparent" } = $props();

async function handleOpenConfig() {
  try {
    await executeAction("open_config_in_terminal");
  } catch {}
}
</script>

{#snippet NavButton({ icon, tab }: { icon: string; tab: string })}
  <button
    type="button"
    class="v-btn-icon v-sidebar-button"
    class:active={nav.activeTab === tab}
    onclick={() => setTab(tab)}
    title={tab}
  >
    <span class="icon">{icon}</span>
  </button>
{/snippet}

<nav
  class="nav-bar"
  class:transparent={variant === "transparent"}
>
  <div class="nav-top-section">
    <div class="nav-group top">
      {@render NavButton({ icon: "house", tab: "home" })}
      {@render NavButton({ icon: "queue_music", tab: "queue" })}
    </div>
  </div>

  <div class="nav-group bottom">
    <button
      type="button"
      class="v-btn-icon v-sidebar-button round"
      onclick={handleOpenConfig}
      title="Open Config"
    >
      <span class="icon">settings</span>
    </button>
  </div>
</nav>

<style>
.nav-bar {
  height: 100%;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: center;
  padding: 8px;
  box-sizing: border-box;
  box-shadow: var(--panel-shadow);
  z-index: 100;
  flex-shrink: 0;
}

.nav-bar:not(.transparent) {
  background-color: var(--bg-panel);
}

.nav-bar.transparent {
  background-color: transparent;
  box-shadow: none;
}

:global(.shader-off) .nav-bar.transparent {
  background-color: var(--bg-panel);
  box-shadow: var(--panel-shadow);
}

.nav-top-section {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
}

.nav-group {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  width: 100%;
}
</style>
