import { sync } from "./sync.svelte.ts";
import { collection } from "./collection.svelte.ts";
import { nav } from "../navigation.svelte.ts";
import { player } from "../modules/player.svelte.ts";
import { persistState } from "./persistence.svelte.ts";
import { prewarmer } from "./prewarmer.svelte.ts";
import { ROOT_TYPE } from "../api.ts";
import type {
  AlbumCardItem,
  FilterState,
  FullAlbum,
  GrouperTreeNode,
  LibraryState,
  SortState,
  WsMessage
} from "../types.ts";

export class ViewState {
  isLoading: boolean = $state(true);
  isConnected: boolean = $state(false);
  homeSubView: "libraries" | "cabinets" = $state("libraries");

  focusedAlbum: FullAlbum | null = $state(null);

  activeLibrary: string = $state("library");
  activeCabinet: string = $state("default");
  activeLibraryFilter: string | null = $state(null);
  activeFilter: FilterState = $state({ key: null, val: null });
  activeSort: SortState = $state({ key: null, order: "default" });
  userSortPreference: string | null = $state(null);
  userSortOrder: string = $state("default");
  activeSidebarGrouper: string | null = $state(null);
  activeShelf: string | null = $state(null);
  activeShelfOrder: string = $state("original");
  activeShelfOrderReverse: boolean = $state(false);
  libraryViewIds: string[] = $state([]);
  shelfViewIds: string[] = $state([]);
  librariesState: Record<string, LibraryState> = $state({});
  libraryVersion: number = $state(0);
  libraryResetVersion: number = $state(0);
  shelfVersion: number = $state(0);
  shelfResetVersion: number = $state(0);
  isShaderEnabled: boolean = $state(true);
  sidebarWidth: number = $state(280);
  isFocusInstant: boolean = $state(false);
  _pendingViewReset: boolean = false;
  _viewSeq: number = 0;
  _sidebarSeq: number = 0;

  libraryAlbums = $derived.by((): AlbumCardItem[] => {
    void this.libraryVersion;
    void collection.dict;
    return collection.mapIdsToAlbums(this.libraryViewIds);
  });

  shelfAlbums = $derived.by((): AlbumCardItem[] => {
    void this.shelfVersion;
    void collection.dict;
    return collection.mapIdsToAlbums(this.shelfViewIds);
  });

  constructor() {
    sync.addEventListener("open", () => {
      this.isConnected = true;
    });
    sync.addEventListener("close", () => {
      this.isConnected = false;
    });
    sync.addEventListener("message", (e: Event) => {
      this.handleMessage((e as CustomEvent<WsMessage>).detail);
    });
  }

  validateAndNormalizeLibrary() {
    if (!collection.availableLibraries[this.activeLibrary]) {
      const fallback =
        collection.manifest.libraries_order?.[0] ||
        Object.keys(collection.availableLibraries)[0] ||
        "library";
      this.activeLibrary = fallback;
      this.loadLibraryState(fallback);
    }

    const libDef = collection.availableLibraries[this.activeLibrary];
    if (libDef) {
      if (
        libDef.allowed_filters &&
        !libDef.allowed_filters.includes(this.activeLibraryFilter || "")
      ) {
        this.activeLibraryFilter =
          libDef.allowed_filters.length > 0 ? libDef.allowed_filters[0] : null;
      } else if (!libDef.allowed_filters || libDef.allowed_filters.length === 0) {
        this.activeLibraryFilter = null;
      }

      if (
        libDef.allowed_groupers &&
        !libDef.allowed_groupers.includes(this.activeSidebarGrouper || "")
      ) {
        this.activeSidebarGrouper =
          libDef.allowed_groupers.length > 0 ? libDef.allowed_groupers[0] : null;
      } else if (!libDef.allowed_groupers || libDef.allowed_groupers.length === 0) {
        this.activeSidebarGrouper = null;
      }

      if (
        this.activeFilter.key &&
        (!libDef.allowed_groupers || !libDef.allowed_groupers.includes(this.activeFilter.key))
      ) {
        this.activeFilter = { key: null, val: null };
      }

      if (
        libDef.allowed_orders &&
        !libDef.allowed_orders.includes(this.userSortPreference || "")
      ) {
        this.userSortPreference =
          libDef.allowed_orders.length > 0 ? libDef.allowed_orders[0] : null;
        this.activeSort = { key: this.userSortPreference, order: this.userSortOrder };
      } else if (!libDef.allowed_orders || libDef.allowed_orders.length === 0) {
        this.userSortPreference = null;
        this.activeSort = { key: null, order: this.userSortOrder };
      }
    }
  }

  handleMessage(json: WsMessage) {
    if (json.event === "media.updated") {
      if (json.type !== ROOT_TYPE) {
        return;
      }
      this.refreshFocused(json.updated, json.removed);
      void this.refreshView(false);
      void this.refreshSidebar();
    } else if (json.event === "logic.updated") {
      collection.initialize().then(() => {
        this.validateAndNormalizeLibrary();
        void this.refreshView(false);
        void this.refreshSidebar();
      });
    } else if (json.event === "config.updated") {
      void this.refreshView(false);
      void this.refreshSidebar();
    }
  }

  refreshFocused(updated: string[], removed: string[]) {
    const focusedId = this.focusedAlbum?.album.id;
    if (!focusedId) {
      return;
    }
    if (removed.includes(focusedId)) {
      this.focusedAlbum = null;
      return;
    }
    if (updated.includes(focusedId)) {
      delete collection.fullAlbumCache[focusedId];
      collection.ensureFullAlbum(focusedId).then((data) => {
        if (data && this.focusedAlbum?.album.id === focusedId) {
          this.focusedAlbum = data;
        }
      });
    }
  }

  get isShaderActive() {
    return this.isShaderEnabled && player.state !== "stop";
  }

  getSidebarGroup(key: string | null): GrouperTreeNode[] {
    if (!key) {
      return [];
    }
    return collection.sidebarGroups.get(key) || [];
  }

  saveCurrentLibraryState() {
    if (!this.activeLibrary) {
      return;
    }
    this.librariesState[this.activeLibrary] = {
      activeLibraryFilter: this.activeLibraryFilter,
      activeFilter: $state.snapshot(this.activeFilter),
      userSortPreference: this.userSortPreference,
      userSortOrder: this.userSortOrder,
      activeSidebarGrouper: this.activeSidebarGrouper,
      activeSort: $state.snapshot(this.activeSort)
    };
  }

  loadLibraryState(key: string) {
    if (!this.librariesState[key]) {
      this.librariesState[key] = {};
    }
    const state = this.librariesState[key];
    const libraryDef = collection.availableLibraries[key] || {};
    const allowedFilters = libraryDef.allowed_filters || [];
    const allowedGroupers = libraryDef.allowed_groupers || [];
    const allowedOrders = libraryDef.allowed_orders || [];

    if (!state.activeLibraryFilter || !allowedFilters.includes(state.activeLibraryFilter)) {
      state.activeLibraryFilter = allowedFilters.length > 0 ? allowedFilters[0] : null;
    }
    if (!state.activeSidebarGrouper || !allowedGroupers.includes(state.activeSidebarGrouper)) {
      state.activeSidebarGrouper = allowedGroupers.length > 0 ? allowedGroupers[0] : null;
    }
    if (!state.userSortPreference || !allowedOrders.includes(state.userSortPreference)) {
      state.userSortPreference = allowedOrders.length > 0 ? allowedOrders[0] : null;
    }
    if (
      state.activeFilter &&
      state.activeFilter.key &&
      !allowedGroupers.includes(state.activeFilter.key)
    ) {
      state.activeFilter = { key: null, val: null };
    }
    if (!state.userSortOrder) {
      state.userSortOrder = "default";
    }
    if (!state.activeSort || state.activeSort.key !== state.userSortPreference) {
      state.activeSort = { key: state.userSortPreference, order: state.userSortOrder };
    }
    if (!state.activeFilter) {
      state.activeFilter = { key: null, val: null };
    }

    this.activeLibraryFilter = state.activeLibraryFilter;
    this.activeSidebarGrouper = state.activeSidebarGrouper;
    this.userSortPreference = state.userSortPreference;
    this.activeSort = { ...state.activeSort };
    this.activeFilter = { ...state.activeFilter };
  }

  async refreshView(resetScroll: boolean = true) {
    this._pendingViewReset = resetScroll;
    this.isLoading = true;
    const seq = ++this._viewSeq;

    try {
      if (nav.activeTab === "home" && this.homeSubView === "cabinets") {
        await this.refreshShelfView(seq);
      } else {
        await this.refreshLibraryView(seq);
      }
    } catch (err) {
      console.error(err);
    } finally {
      if (seq === this._viewSeq) {
        this.isLoading = false;
        this._pendingViewReset = false;
      }
    }
  }

  async refreshLibraryView(seq: number) {
    const ids = await collection.loadItems({
      library: this.activeLibrary,
      library_filter: this.activeLibraryFilter ?? undefined,
      group: this.activeFilter.key ?? undefined,
      group_val: this.activeFilter.val ?? undefined,
      sort: this.activeSort.key ?? undefined,
      reverse: this.activeSort.order === "reverse"
    });

    if (seq !== this._viewSeq) {
      return;
    }

    this.libraryViewIds = ids;
    this.libraryVersion++;
    if (this._pendingViewReset) {
      this.libraryResetVersion++;
    }
    prewarmer.refresh();
  }

  async refreshShelfView(seq: number) {
    const visibleShelves = collection.getVisibleShelvesForCabinet(this.activeCabinet);
    if (visibleShelves.length > 0 && !visibleShelves.some((s) => s.key === this.activeShelf)) {
      this.activeShelf = visibleShelves[0].key;
    }
    const visibleOrders = collection.getVisibleOrdersForCabinet(this.activeCabinet);
    if (
      this.activeShelfOrder !== "original" &&
      !visibleOrders.some((o) => o.key === this.activeShelfOrder)
    ) {
      this.activeShelfOrder = "original";
    }

    const shelfKey =
      this.activeShelf ||
      (collection.manifest.shelves_order && collection.manifest.shelves_order[0]) ||
      Object.keys(collection.availableShelves)[0];

    const ids = await collection.loadItems({
      shelf: shelfKey,
      sort: this.activeShelfOrder === "original" ? undefined : this.activeShelfOrder,
      reverse: this.activeShelfOrderReverse
    });

    if (seq !== this._viewSeq) {
      return;
    }

    this.shelfViewIds = ids;
    this.shelfVersion++;
    if (this._pendingViewReset) {
      this.shelfResetVersion++;
    }
    prewarmer.refresh();
  }

  async refreshSidebar() {
    if (!this.activeSidebarGrouper) {
      return;
    }

    const seq = ++this._sidebarSeq;
    try {
      await collection.loadGroups(this.activeSidebarGrouper, {
        library: this.activeLibrary,
        library_filter: this.activeLibraryFilter ?? undefined
      });
    } catch (err) {
      if (seq === this._sidebarSeq) {
        console.error(err);
      }
    }
  }

  setLibrary(key: string) {
    this.saveCurrentLibraryState();
    this.activeLibrary = key;
    this.loadLibraryState(key);
    this.refreshView(true);
    this.refreshSidebar();
    this.persistState();
  }

  setCabinet(key: string) {
    this.activeCabinet = key;
    const visibleShelves = collection.getVisibleShelvesForCabinet(key);
    this.activeShelf = visibleShelves.length > 0 ? visibleShelves[0].key : null;
    this.activeShelfOrder = "original";
    this.activeShelfOrderReverse = false;
    this.focusedAlbum = null;
    this.refreshView(true);
    this.persistState();
  }

  setLibraryFilter(key: string) {
    this.activeLibraryFilter = key;
    this.activeFilter = { key: null, val: null };
    this.refreshView(true);
    this.refreshSidebar();
    this.persistState();
  }

  setShelf(key: string) {
    this.activeShelf = key;
    this.focusedAlbum = null;
    this.refreshView(true);
    this.persistState();
  }

  setShelfOrder(key: string) {
    this.activeShelfOrder = key;
    this.refreshView(true);
    this.persistState();
  }

  toggleShelfOrderDirection() {
    this.activeShelfOrderReverse = !this.activeShelfOrderReverse;
    this.refreshView(true);
    this.persistState();
  }

  setSidebarGrouper(key: string) {
    this.activeSidebarGrouper = key;
    if (this.activeFilter.key !== key) {
      this.activeFilter = { key: null, val: null };
      this.focusedAlbum = null;
      this.refreshView(true);
    }
    this.refreshSidebar();
    this.persistState();
  }

  applyFilter(key: string, val: string) {
    if (this.activeFilter.key === key && this.activeFilter.val === val) {
      this.activeFilter = { key: null, val: null };
    } else {
      this.activeFilter = { key, val };
    }
    this.focusedAlbum = null;
    this.activeSort = { key: this.userSortPreference, order: this.userSortOrder };
    this.refreshView(true);
    this.persistState();
  }

  setUserSort(key: string) {
    this.userSortPreference = key;
    this.activeSort = { key, order: this.userSortOrder };
    this.refreshView(true);
    this.persistState();
  }

  toggleSortOrder() {
    this.userSortOrder = this.userSortOrder === "default" ? "reverse" : "default";
    this.activeSort = { key: this.userSortPreference, order: this.userSortOrder };
    this.refreshView(true);
    this.persistState();
  }

  async setFocus(album: { id: string }, instant: boolean = false) {
    if (this.focusedAlbum?.album.id === album.id) {
      this.isFocusInstant = instant;
      return;
    }

    this.isFocusInstant = instant;

    const cached = collection.fullAlbumCache[album.id];
    if (cached) {
      this.focusedAlbum = cached;
      return;
    }

    const dictEntry = collection.dict[album.id];
    this.focusedAlbum = {
      album: {
        ...(dictEntry || { id: album.id }),
        tracks: []
      }
    };

    const full = await collection.ensureFullAlbum(album.id);
    if (full && this.focusedAlbum?.album.id === album.id) {
      this.focusedAlbum = full;
    }
  }

  closeFocus() {
    this.focusedAlbum = null;
    this.isFocusInstant = false;
  }

  toggleShader() {
    this.isShaderEnabled = !this.isShaderEnabled;
    this.persistState();
  }

  persistState() {
    persistState(this);
  }
}

export const view = new ViewState();
