import { SvelteMap } from "svelte/reactivity";
import { fetchMedia } from "../api.ts";
import { sync } from "./sync.svelte.ts";
import { updatePlayerState } from "../modules/player.svelte.ts";
import { config as globalConfig } from "../config.svelte.ts";
import type {
  ActionDef,
  AlbumCardItem,
  AlbumDictEntry,
  AlbumImageEntry,
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

const DEFAULT_THUMBNAIL_SIZE = 200;
const DEFAULT_THUMBNAIL_FILTER = "lanczos";

/**
 * The grid thumbnail URL and the prewarm request must agree on size and
 * filter: the cache file is keyed by both, so any mismatch misses the cache.
 */
function thumbnailParams(): { size: number; filter: string } {
  const imageConfig = globalConfig.album_grid.album_card.cover;
  return {
    size: imageConfig.size || DEFAULT_THUMBNAIL_SIZE,
    filter: imageConfig.filter || DEFAULT_THUMBNAIL_FILTER
  };
}

function mainImageAddress(
  album: { images?: Record<string, AlbumImageEntry[]> } | null | undefined
): string {
  return album?.images?.main?.[0]?.address || "";
}

/**
 * Prewarms derived thumbnails. Without addresses the whole library is swept,
 * so single-album updates pass only what they touched.
 */
function triggerImagePrewarm(addresses?: string[]) {
  const { size, filter } = thumbnailParams();
  const target = addresses
    ? { addresses }
    : { names: ["main"] };
  fetch("/api/images/cache", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...target, format: "bmp", size, filter })
  }).catch(() => {});
}

class CollectionStore {
  dict: Record<string, AlbumDictEntry> = $state({});
  sidebarShelves: Record<string, string[]> = $state({});
  libraryViewIds: string[] = $state([]);
  sidebarGroups: SvelteMap<string, GrouperTreeNode[]> = $state(new SvelteMap());
  fullAlbumCache: Record<string, FullAlbum> = $state({});
  actions: Record<string, ActionDef> = $state({});
  manifest: LogicManifest = $state({
    filters: {},
    libraries: {},
    groupers: {},
    orders: {},
    shelves: {},
    cabinets: {}
  });

  constructor() {
    sync.addEventListener("message", (e: Event) =>
      this.handleMessage((e as CustomEvent<WsMessage>).detail)
    );
  }

  handleMessage(json: WsMessage) {
    if (json.type === "INIT") {
      if (json.shelves) {
        this.sidebarShelves = json.shelves;
      }
      this.dict = json.media?.album || {};
      if (json.actions) {
        this.actions = json.actions;
      }
      if (json.manifest) {
        this.manifest = json.manifest;
      }
      triggerImagePrewarm();
    } else if (json.type === "VIEW_DATA") {
      this.libraryViewIds = json.ids || [];
    } else if (json.type === "GROUP_RESULT") {
      const newMap = new SvelteMap(this.sidebarGroups);
      newMap.set(json.key, json.result);
      this.sidebarGroups = newMap;
    } else if (json.type === "MEDIA_UPDATED") {
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

      const addresses = Object.values(json.updated || {})
        .map(mainImageAddress)
        .filter((address) => address.length > 0);
      if (addresses.length > 0) {
        triggerImagePrewarm(addresses);
      }
    } else if (json.type === "LOGIC_UPDATE") {
      if (json.actions) {
        this.actions = json.actions;
      }
      if (json.manifest) {
        this.manifest = json.manifest;
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
      const info = a.objects?.info;
      albums.push({
        id: a.id,
        title: a.tags?.album,
        artist: a.tags?.albumartist,
        images: a.images,
        total_discs: Number(info?.total_discs) || 1,
        total_tracks: Number(info?.total_tracks) || 0,
        duration_formatted: info?.duration_formatted,
        virtual: info?.virtual,
        tags: a.tags,
        colors: a.objects?.colors ?? null
      });
    }
    return albums;
  }

  getThumbnailUrl(
    album: AlbumCardItem | AlbumDictEntry | null | undefined
  ): string {
    const address = mainImageAddress(album);
    if (!address) {
      return "";
    }
    const { size, filter } = thumbnailParams();
    return `/api/images/${address}.bmp?size=${size}&filter=${filter}`;
  }

  async ensureFullAlbum(id: string): Promise<FullAlbum | null> {
    if (!id) {
      return null;
    }
    if (this.fullAlbumCache[id]) {
      return this.fullAlbumCache[id];
    }
    try {
      const data = await fetchMedia("album", id);
      if (data) {
        this.fullAlbumCache[id] = data;
        return data;
      }
    } catch (err) {
      console.error(err);
    }
    return null;
  }

  /** Interfaces only render a control when the config declares its action. */
  hasAction(name: string): boolean {
    return Boolean(this.actions[name]);
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
