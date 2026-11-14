import { SvelteMap, SvelteSet } from "svelte/reactivity";
import { sync } from "./sync.svelte.ts";
import { player, updatePlayerState } from "../modules/player.svelte.ts";
import { config as globalConfig, updateConfig } from "../config.svelte.ts";
import {
  ROOT_TYPE,
  fetchGroups,
  fetchItem,
  fetchMedia,
  fetchItems,
  type ItemsQuery
} from "../api.ts";
import type {
  AlbumCardItem,
  AlbumDictEntry,
  CabinetDef,
  CabinetListItem,
  DropdownItem,
  FilterDef,
  FullAlbum,
  GrouperDef,
  GrouperTreeNode,
  LibraryDef,
  LibraryListItem,
  LogicManifest,
  OrderDef,
  ShelfDef,
  ShelfListItem,
  WsMessage
} from "../types.ts";

const CARD_VIEW = "tags,objects,images,location";
const COVER_SIZES = [200, 619];

function triggerCoverPrewarm() {
  for (const size of COVER_SIZES) {
    fetch("/api/internal/cache_images", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        names: ["main"],
        format: "bmp",
        size,
        filter: "catmullrom"
      })
    }).catch(() => {});
  }
}

class CollectionStore {
  dict: Record<string, AlbumDictEntry> = $state({});
  sidebarGroups: SvelteMap<string, GrouperTreeNode[]> = $state(new SvelteMap());
  fullAlbumCache: Record<string, FullAlbum> = $state({});
  manifest: LogicManifest = $state({
    filters: {},
    libraries: {},
    groupers: {},
    orders: {},
    shelves: {},
    cabinets: {}
  });
  config: Record<string, unknown> = $state({});
  types: Record<string, { dir: string; count: number; children: Record<string, string> }> =
    $state({});
  actions: Record<string, { label?: string; description?: string }> = $state({});
  private pendingEntries = new SvelteSet<string>();

  constructor() {
    sync.addEventListener("message", (e: Event) =>
      this.handleMessage((e as CustomEvent<WsMessage>).detail)
    );
  }

  async initialize() {
    const discovery = await fetchMedia(ROOT_TYPE);
    this.manifest = discovery.logic as LogicManifest;
    this.types = discovery.types;
    this.actions = discovery.actions;
    this.config = { ...this.config, ...discovery.config };

    triggerCoverPrewarm();
  }

  handleMessage(json: WsMessage) {
    if (json.event === "media.updated") {
      if (json.type !== ROOT_TYPE) {
        return;
      }
      for (const id of json.removed) {
        delete this.dict[id];
        delete this.fullAlbumCache[id];
      }
      for (const id of json.updated) {
        delete this.dict[id];
        delete this.fullAlbumCache[id];
      }
      if (player.currentAlbumId) {
        this.ensureAlbumEntry(player.currentAlbumId);
      }
      triggerCoverPrewarm();
    } else if (json.event === "config.updated") {
      if (json.config) {
        this.config = { ...this.config, ...json.config };
        updateConfig(json.config);
      }
    } else if (json.event === "playback.updated") {
      updatePlayerState(json);
      const albumIds: string[] = [];
      const addAlbumId = (id: string | null | undefined) => {
        if (id && !albumIds.includes(id)) {
          albumIds.push(id);
        }
      };
      addAlbumId(json.album_id);
      for (const item of json.queue ?? []) {
        addAlbumId(item.album_id);
      }
      for (const id of albumIds) {
        if (!this.dict[id]) {
          this.ensureAlbumEntry(id);
        }
      }
    }
  }

  async loadItems(query: ItemsQuery): Promise<string[]> {
    const response = await fetchItems<AlbumDictEntry>(ROOT_TYPE, {
      ...query,
      view: query.view ?? CARD_VIEW
    });

    for (const item of response.items) {
      const entity = item[ROOT_TYPE];
      if (entity) {
        this.dict[entity.id] = entity;
      }
    }

    triggerCoverPrewarm();
    return response.ids;
  }

  async loadGroups(
    grouper: string,
    query: { library?: string; library_filter?: string }
  ): Promise<GrouperTreeNode[]> {
    const tree = (await fetchGroups(ROOT_TYPE, grouper, query)) as GrouperTreeNode[];
    const next = new SvelteMap(this.sidebarGroups);
    next.set(grouper, tree);
    this.sidebarGroups = next;
    return tree;
  }

  mapIdsToAlbums(ids: string[]): AlbumCardItem[] {
    return ids
      .map((id) => this.dict[id])
      .filter((album): album is AlbumDictEntry => Boolean(album));
  }

  getThumbnailUrl(album: AlbumCardItem | null | undefined): string {
    const address = album?.images?.main?.[0]?.address;
    if (!address) {
      return "";
    }
    const algo = globalConfig.album_grid.album_card.cover.filter || "lanczos";
    const size = globalConfig.album_grid.album_card.cover.size || 200;
    return `/api/images/${address}.bmp?size=${size}&filter=${algo}`;
  }

  async ensureAlbumEntry(id: string): Promise<AlbumDictEntry | null> {
    if (this.dict[id]) {
      return this.dict[id];
    }
    if (this.pendingEntries.has(id)) {
      return null;
    }

    this.pendingEntries.add(id);
    try {
      const full = await this.ensureFullAlbum(id);
      if (!full) {
        return null;
      }
      this.dict[id] = full.album;
      return full.album;
    } finally {
      this.pendingEntries.delete(id);
    }
  }

  async ensureFullAlbum(id: string): Promise<FullAlbum | null> {
    if (!id) {
      return null;
    }
    if (this.fullAlbumCache[id]) {
      return this.fullAlbumCache[id];
    }
    try {
      const data = await fetchItem<FullAlbum>(ROOT_TYPE, id);
      this.fullAlbumCache[id] = data;
      return data;
    } catch (err) {
      console.error(err);
    }
    return null;
  }

  get availableFilters(): Record<string, FilterDef> {
    return this.manifest.filters || {};
  }
  get availableLibraries(): Record<string, LibraryDef> {
    return this.manifest.libraries || {};
  }
  get availableGroupers(): Record<string, GrouperDef> {
    return this.manifest.groupers || {};
  }
  get availableOrders(): Record<string, OrderDef> {
    return this.manifest.orders || {};
  }
  get availableShelves(): Record<string, ShelfDef> {
    return this.manifest.shelves || {};
  }
  get availableCabinets(): Record<string, CabinetDef> {
    return this.manifest.cabinets || {};
  }

  get librariesList(): LibraryListItem[] {
    const order = this.manifest.libraries_order || Object.keys(this.availableLibraries);
    return order.map((k: string) => ({ key: k, ...(this.availableLibraries[k] || { label: k }) }));
  }

  get shelvesList(): ShelfListItem[] {
    const order = this.manifest.shelves_order || Object.keys(this.availableShelves);
    return order.map((k: string) => ({ key: k, ...(this.availableShelves[k] || { label: k }) }));
  }

  get cabinetsList(): CabinetListItem[] {
    const order = this.manifest.cabinets_order || Object.keys(this.availableCabinets);
    return order.map((k: string) => ({ key: k, ...(this.availableCabinets[k] || { label: k }) }));
  }

  getVisibleFilters(activeLibrary: string): DropdownItem[] {
    const library = this.availableLibraries[activeLibrary];
    const order = this.manifest.filters_order || Object.keys(this.availableFilters);
    if (library && library.allowed_filters) {
      return library.allowed_filters
        .filter((k: string) => this.availableFilters[k])
        .map((k: string) => ({ key: k, label: this.availableFilters[k].label || k }));
    }
    return order
      .filter((k: string) => this.availableFilters[k])
      .map((k: string) => ({ key: k, label: this.availableFilters[k].label || k }));
  }

  getVisibleGroupers(activeLibrary: string): DropdownItem[] {
    const library = this.availableLibraries[activeLibrary];
    const order = this.manifest.groupers_order || Object.keys(this.availableGroupers);
    if (library && library.allowed_groupers) {
      return library.allowed_groupers
        .filter((k: string) => this.availableGroupers[k])
        .map((k: string) => ({ key: k, label: this.availableGroupers[k].label || k }));
    }
    return order
      .filter((k: string) => this.availableGroupers[k])
      .map((k: string) => ({ key: k, label: this.availableGroupers[k].label || k }));
  }

  getVisibleOrders(activeLibrary: string): DropdownItem[] {
    const library = this.availableLibraries[activeLibrary];
    const order = this.manifest.orders_order || Object.keys(this.availableOrders);
    if (library && library.allowed_orders) {
      return library.allowed_orders
        .filter((k: string) => this.availableOrders[k])
        .map((k: string) => ({ key: k, label: this.availableOrders[k].label || k }));
    }
    return order
      .filter((k: string) => this.availableOrders[k])
      .map((k: string) => ({ key: k, label: this.availableOrders[k].label || k }));
  }

  getVisibleShelvesForCabinet(activeCabinet: string): DropdownItem[] {
    const cabinet = this.availableCabinets[activeCabinet];
    const order = this.manifest.shelves_order || Object.keys(this.availableShelves);
    if (cabinet && cabinet.allowed_shelves && cabinet.allowed_shelves.length > 0) {
      return cabinet.allowed_shelves
        .filter((k: string) => this.availableShelves[k])
        .map((k: string) => ({ key: k, label: this.availableShelves[k].label || k }));
    }
    return order
      .filter((k: string) => this.availableShelves[k])
      .map((k: string) => ({ key: k, label: this.availableShelves[k].label || k }));
  }

  getVisibleOrdersForCabinet(activeCabinet: string): DropdownItem[] {
    const cabinet = this.availableCabinets[activeCabinet];
    if (!cabinet || !cabinet.allowed_orders || cabinet.allowed_orders.length === 0) {
      return [];
    }
    return cabinet.allowed_orders
      .filter((k: string) => this.availableOrders[k])
      .map((k: string) => ({ key: k, label: this.availableOrders[k].label || k }));
  }
}

export const collection = new CollectionStore();
