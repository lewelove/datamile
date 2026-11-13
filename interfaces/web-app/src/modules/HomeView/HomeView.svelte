<script lang="ts">
import { view } from "../../library/view.svelte.ts";
import NavigationBar from "../NavigationBar.svelte";
import LibrariesView from "./LibrariesView.svelte";
import CabinetsView from "./CabinetsView.svelte";
import Sidebar from "./Sidebar.svelte";
import ModalDrawer from "./ModalDrawer/ModalDrawer.svelte";

let isResizing = $state(false);

let isModalVisible = $derived(Boolean(view.focusedAlbum));

function startResizing() {
  isResizing = true;
  const move = (e: MouseEvent) => {
    const w = window.innerWidth;
    view.sidebarWidth = Math.max(140, Math.min(w - e.clientX, 400));
  };
  const up = () => {
    isResizing = false;
    view.persistState();
    window.removeEventListener("mousemove", move);
    window.removeEventListener("mouseup", up);
  };
  window.addEventListener("mousemove", move);
  window.addEventListener("mouseup", up);
}
</script>

<div class="home-view-container" style:--sidebar-width="{view.sidebarWidth}px">
  <NavigationBar />

  <div class="workspace">
    <section class="plane home-grid" class:resizing={isResizing}>
      {#if view.homeSubView === "cabinets"}
        <CabinetsView
          activeAlbumId={view.focusedAlbum?.album.id}
          onfocus={(album) => view.setFocus(album)}
        />
      {:else}
        <LibrariesView
          activeAlbumId={view.focusedAlbum?.album.id}
          onfocus={(album) => view.setFocus(album)}
        />
      {/if}
    </section>

    <aside class="sidebar-shell right" class:dormant={isModalVisible}>
      <div class="sidebar-panel">
        <button type="button" class="sidebar-resizer" onmousedown={startResizing} aria-label="Resize sidebar"
        ></button>
        <div class="sidebar-inner">
          <Sidebar />
        </div>
      </div>
    </aside>
  </div>

  {#if view.focusedAlbum}
    <div class="modal-layer">
      <ModalDrawer album={view.focusedAlbum} onclose={() => view.closeFocus()} />
    </div>
  {/if}
</div>

<style>
.home-view-container {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  overflow: hidden;
  position: relative;
}

.workspace {
  flex: 1;
  position: relative;
  height: 100%;
  overflow: hidden;
  min-width: 0;
}

.plane {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  overflow: hidden;
}

.home-grid {
  z-index: 1;
  left: 0;
  width: calc(100% - (var(--sidebar-width)));
  transition: width 0.25s cubic-bezier(0.2, 0, 0, 1);
}

.home-grid.resizing {
  transition: none;
}

.sidebar-shell {
  position: absolute;
  top: 0;
  bottom: 0;
  z-index: 100;
  pointer-events: none;
}

.sidebar-shell.dormant {
  pointer-events: none !important;
}

.sidebar-shell.right {
  right: 0;
  width: var(--sidebar-width);
}

.sidebar-panel {
  position: absolute;
  inset: 0;
  background-color: var(--bg-panel);
  pointer-events: auto;
  display: flex;
  flex-direction: row;
  box-sizing: border-box;
  box-shadow: var(--panel-shadow);
  overflow: visible;
}

.sidebar-resizer {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 6px;
  cursor: col-resize;
  z-index: 120;
  transform: translateX(-50%);
  pointer-events: auto;
  background: transparent;
  border: none;
  padding: 0;
  margin: 0;
}

.sidebar-inner {
  flex: 1;
  overflow: hidden;
}

.modal-layer {
  position: absolute;
  inset: 0;
  z-index: 150;
}
</style>
