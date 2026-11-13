import { SvelteMap } from "svelte/reactivity";
import { sync } from "./sync.svelte.ts";
import { updatePlayerState } from "../modules/player.svelte.ts";
import { config as globalConfig, updateConfig } from "../config.svelte.ts";
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

function triggerCoverPrewarm() {
  const sizes = [200, 619];
  for (const size of sizes) {
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
  trackPathMap: Record<string, unknown> = $state({});
  sidebarShelves: Record<string, string[]> = $state({});
  libraryViewIds: string[] = $state([]);
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

  constructor() {
    sync.addEventListener("message", (e: Event) =>
      this.handleMessage((e as CustomEvent<WsMessage>).detail)
    );
  }

  handleMessage(json: WsMessage) {
    if (json.type === "INIT_DICT") {
      if (json.shelves) {
        this.sidebarShelves = json.shelves;
      }
      this.dict = json.dict || {};
      this.trackPathMap = json.trackMap || {};
      if (json.manifest) {
        this.manifest = json.manifest;
      }
      if (json.config) {
        this.config = { ...this.config, ...json.config };
      }
      triggerCoverPrewarm();
    } else if (json.type === "VIEW_DATA") {
      this.libraryViewIds = json.ids || [];
    } else if (json.type === "GROUP_RESULT") {
      const newMap = new SvelteMap(this.sidebarGroups);
      newMap.set(json.key, json.result);
      this.sidebarGroups = newMap;
    } else if (json.type === "ALBUM_REMOVED") {
      if (json.shelves) {
        this.sidebarShelves = json.shelves;
      }
      delete this.dict[json.id];
      delete this.fullAlbumCache[json.id];
      triggerCoverPrewarm();
    } else if (json.type === "ALBUM_UPDATED") {
      if (json.shelves) {
        this.sidebarShelves = json.shelves;
      }
      if (json.old_id && json.old_id !== json.id) {
        delete this.dict[json.old_id];
        delete this.fullAlbumCache[json.old_id];
      }
      if (json.dictEntry && Object.keys(json.dictEntry).length > 0) {
        this.dict[json.id] = json.dictEntry;
      } else {
        delete this.dict[json.id];
      }
      delete this.fullAlbumCache[json.id];
      triggerCoverPrewarm();
    } else if (json.type === "ALBUMS_UPDATED") {
      if (json.shelves) {
        this.sidebarShelves = json.shelves;
      }
      if (json.removed) {
        for (const id of json.removed) {
          if (!json.updated || !json.updated[id]) {
            delete this.dict[id];
            delete this.fullAlbumCache[id];
          }
        }
      }
      if (json.updated) {
        for (const [id, dictEntry] of Object.entries(json.updated)) {
          if (dictEntry && Object.keys(dictEntry).length > 0) {
            this.dict[id] = dictEntry;
          } else {
            delete this.dict[id];
          }
          delete this.fullAlbumCache[id];
        }
      }
      triggerCoverPrewarm();
    } else if (json.type === "LOGIC_UPDATE") {
      if (json.manifest) {
        this.manifest = json.manifest;
      }
    } else if (json.type === "CONFIG_UPDATE") {
      if (json.config) {
        this.config = { ...this.config, ...json.config };
        updateConfig(json.config);
      }
    } else if (json.type === "MPD_STATUS") {
      updatePlayerState(json);
    }
  }

  mapIdsToAlbums(ids: string[]): AlbumCardItem[] {
    const albums: AlbumCardItem[] = [];
    for (const id of ids) {
      const a = this.dict[id];
      if (!a) {
        continue;
      }
      albums.push({
        id: a.id,
        title: a.album,
        artist: a.albumartist,
        cover_hash: a.cover_hash,
        covers: a.covers,
        total_discs: a.total_discs,
        total_tracks: a.total_tracks,
        duration_formatted: a.duration_formatted,
        virtual: a.virtual,
        keys: a.keys,
        colors: a.colors
      });
    }
    return albums;
  }

  getThumbnailUrl(album: { cover_hash?: string | null } | null | undefined): string {
    if (!album || !album.cover_hash) {
      return "";
    }
    const algo = globalConfig.album_grid.album_card.cover.filter || "lanczos";
    const size = globalConfig.album_grid.album_card.cover.size || 200;
    return `/api/images/${album.cover_hash}.bmp?size=${size}&filter=${algo}`;
  }

  async ensureFullAlbum(id: string): Promise<FullAlbum | null> {
    if (!id) {
      return null;
    }
    if (this.fullAlbumCache[id]) {
      return this.fullAlbumCache[id];
    }
    try {
      const res = await fetch(`/api/album/${encodeURIComponent(id)}`);
      if (res.ok) {
        const data = (await res.json()) as FullAlbum;
        data.id = id;
        this.fullAlbumCache[id] = data;
        return data;
      }
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
