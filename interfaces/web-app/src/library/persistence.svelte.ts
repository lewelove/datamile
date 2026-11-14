import { nav } from "../navigation.svelte.ts";
import { WEB_APP_NAMESPACE, saveState } from "../api.ts";
import type { ViewState } from "./view.svelte.ts";
import type { PersistedState } from "../types.ts";

export function applyPersistedState(state: PersistedState, view: ViewState) {
  nav.activeTab = state.activeTab || "home";
  view.homeSubView = state.homeSubView || "libraries";
  view.librariesState = state.librariesState || {};
  view.activeLibrary = state.activeLibrary || "library";

  view.loadLibraryState(view.activeLibrary);

  if (state.activeCabinet !== undefined) {
    view.activeCabinet = state.activeCabinet;
  }
  if (state.activeLibraryFilter !== undefined) {
    view.activeLibraryFilter = state.activeLibraryFilter;
  }
  if (state.groupKey !== undefined) {
    view.activeSidebarGrouper = state.groupKey;
  }
  if (state.filter !== undefined) {
    view.activeFilter = state.filter;
  }
  if (state.sortKey !== undefined) {
    view.userSortPreference = state.sortKey;
    view.activeSort = { key: state.sortKey, order: view.activeSort.order };
  }
  if (state.sortOrder !== undefined) {
    view.userSortOrder = state.sortOrder;
    view.activeSort = { key: view.userSortPreference, order: state.sortOrder };
  }

  view.activeShelf = state.activeShelf || null;
  if (state.activeShelfOrder !== undefined) {
    view.activeShelfOrder = state.activeShelfOrder;
  }
  if (state.activeShelfOrderReverse !== undefined) {
    view.activeShelfOrderReverse = state.activeShelfOrderReverse;
  }

  view.isShaderEnabled = state.isShaderEnabled ?? true;
  view.sidebarWidth = state.sidebarWidth || 280;
}

export function persistState(view: ViewState) {
  view.saveCurrentLibraryState();
  saveState(WEB_APP_NAMESPACE, {
    activeTab: nav.activeTab,
    homeSubView: view.homeSubView,
    activeLibrary: view.activeLibrary,
    activeCabinet: view.activeCabinet,
    librariesState: $state.snapshot(view.librariesState),
    activeLibraryFilter: view.activeLibraryFilter,
    sortKey: view.userSortPreference,
    sortOrder: view.userSortOrder,
    groupKey: view.activeSidebarGrouper,
    filter: $state.snapshot(view.activeFilter),
    activeShelf: view.activeShelf,
    activeShelfOrder: view.activeShelfOrder,
    activeShelfOrderReverse: view.activeShelfOrderReverse,
    isShaderEnabled: view.isShaderEnabled,
    sidebarWidth: view.sidebarWidth
  }).catch((err) => console.error(err));
}
