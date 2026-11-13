<script lang="ts">
import { onMount } from "svelte";
import { view } from "./library/view.svelte.ts";
import { initApp } from "./library/init.ts";
import { nav, setTab } from "./navigation.svelte.ts";
import { executeAction } from "./api.ts";
import { player } from "./modules/player.svelte.ts";

import HomeView from "./modules/HomeView/HomeView.svelte";
import QueueView from "./modules/QueueView/QueueView.svelte";

const tabOrder: Record<string, number> = { home: 1, queue: 2 };
let currentTab = $state(nav.activeTab);
let retentionTab: string | null = $state(null);
let instantTab: string | null = $state(null);

$effect(() => {
  if (nav.activeTab !== currentTab) {
    const oldTab = currentTab;
    const newTab = nav.activeTab;

    if (tabOrder[newTab] > tabOrder[oldTab]) {
      retentionTab = oldTab;
      instantTab = null;
      setTimeout(() => {
        if (retentionTab === oldTab) retentionTab = null;
      }, 100);
    } else {
      retentionTab = null;
      instantTab = newTab;
      setTimeout(() => {
        if (instantTab === newTab) instantTab = null;
      }, 100);
    }

    currentTab = newTab;
  }
});

let isHomeActive = $derived(currentTab === "home");
let isQueueActive = $derived(currentTab === "queue");

let isHomeVisible = true;
let isQueueVisible = $derived(currentTab === "queue" || retentionTab === "queue");

let isQueueInstant = $derived(instantTab === "queue");

let isModalVisible = $derived(!!view.focusedAlbum);

function handleKeydown(e: KeyboardEvent) {
  if (["INPUT", "TEXTAREA"].includes(document.activeElement?.tagName ?? "")) return;

  const code = e.code;
  const key = e.key;

  if ((e.ctrlKey || e.metaKey) && (key === "," || code === "Comma")) {
    e.preventDefault();
    e.stopPropagation();
    executeAction("open_config_in_terminal", "").catch(() => {});
    return;
  }

  if ((e.ctrlKey || e.metaKey) && (key === "t" || key === "T" || code === "KeyT")) {
    e.preventDefault();
    e.stopPropagation();
    const targetId = isModalVisible
      ? view.focusedAlbum?.album.id
      : nav.activeTab === "queue"
        ? player.currentAlbumId
        : null;

    if (targetId) {
      executeAction("open_terminal", targetId).catch(() => {});
    }
    return;
  }

  if ((e.ctrlKey || e.metaKey) && (key === "d" || key === "D" || code === "KeyD")) {
    e.preventDefault();
    e.stopPropagation();
    const targetId = isModalVisible
      ? view.focusedAlbum?.album.id
      : nav.activeTab === "queue"
        ? player.currentAlbumId
        : null;

    if (targetId) {
      executeAction("open_folder", targetId).catch(() => {});
    }
    return;
  }

  if (code === "Space") {
    e.preventDefault();
    e.stopPropagation();
    fetch("/api/toggle-pause", { method: "POST" }).catch(() => {});
    return;
  }

  if (key === "Enter" || code === "NumpadEnter") {
    e.preventDefault();
    e.stopPropagation();
    return;
  }

  const lowerKey = key.toLowerCase();

  if (lowerKey === "escape" && isModalVisible) {
    e.preventDefault();
    e.stopPropagation();
    view.closeFocus();
    return;
  }

  if (lowerKey === "1" || lowerKey === "h") {
    if (nav.activeTab !== "home") {
      setTab("home");
    } else {
      view.refreshView(false);
      view.persistState();
    }
  }
  if (lowerKey === "2" || lowerKey === "q") {
    setTab("queue");
  }
  if (lowerKey === "s") {
    view.homeSubView = "cabinets";
    if (nav.activeTab !== "home") {
      setTab("home");
    } else {
      view.refreshView(false);
      view.persistState();
    }
  }
}

function handleFocusIn(e: FocusEvent) {
  if (!e.target) return;
  const tag = (e.target as HTMLElement).tagName;
  if (tag !== "INPUT" && tag !== "TEXTAREA") {
    if (e.target !== document.body) {
      document.body.focus({ preventScroll: true });
    }
  }
}

onMount(() => {
  initApp();
  window.addEventListener("keydown", handleKeydown, { capture: true });
  window.addEventListener("focusin", handleFocusIn, { capture: true });
  return () => {
    window.removeEventListener("keydown", handleKeydown, { capture: true });
    window.removeEventListener("focusin", handleFocusIn, { capture: true });
  };
});
</script>

<main tabindex="-1">
  <div
    class="view-layer home"
    class:visible={isHomeVisible}
    class:active={isHomeActive}
    aria-hidden={!isHomeActive}
  >
    <HomeView />
  </div>

  <div
    class="view-layer queue"
    class:visible={isQueueVisible}
    class:active={isQueueActive}
    class:instant={isQueueInstant}
    aria-hidden={!isQueueActive}
  >
    <QueueView />
  </div>
</main>

<style>
:root {
  --nav-height: 80px;
  --trigger-size: 24px;
}

main {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background-color: var(--bg-main);
  outline: none;
}

.view-layer {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  overflow: hidden;
  opacity: 0;
  visibility: hidden;
  pointer-events: none;
}

.view-layer.visible {
  opacity: 1;
  visibility: visible;
}

.view-layer.active {
  pointer-events: auto;
}

.view-layer.home {
  z-index: 1;
  background-color: var(--bg-main);
}

.view-layer.queue {
  z-index: 2;
  background-color: var(--bg-panel);
}

.view-layer.queue,
.view-layer.queue.visible {
  transition:
    opacity 50ms ease,
    visibility 50ms ease;
}
</style>
