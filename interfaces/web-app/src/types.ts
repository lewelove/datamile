export interface AlbumImageEntry {
  address: string;
  width?: number;
  height?: number;
  fit?: string;
  filter?: string;
  source?: {
    file?: {
      path?: string;
      hash?: string;
      mtime?: number;
      byte_size?: number;
    };
    info?: {
      width?: number;
      height?: number;
      format?: string;
    };
  };
}

export interface AlbumColors {
  foreground?: string | null;
  background?: string[] | null;
}

export interface AlbumInfoData {
  virtual?: boolean;
  total_discs?: number | string;
  total_tracks?: number | string;
  duration_milliseconds?: number;
  duration_formatted?: string;
}

export interface AlbumTags {
  album?: string;
  albumartist?: string;
  date?: string;
  [key: string]: unknown;
}

export interface AlbumObjects {
  info?: AlbumInfoData;
  colors?: AlbumColors | null;
  [key: string]: unknown;
}

export interface TrackTags {
  tracknumber?: number | string;
  discnumber?: number | string;
  title?: string;
  artist?: string;
  lyrics?: string;
  instrumental?: boolean;
  [key: string]: unknown;
}

export interface TrackObjects {
  info?: TrackPhysicsInfo;
  [key: string]: unknown;
}

export interface NodeLocation {
  kind?: "dir" | "file";
  path?: string;
  mtime?: number;
  byte_size?: number;
}

export interface AlbumLockData {
  id?: string;
  location?: NodeLocation;
  tags?: AlbumTags;
  objects?: AlbumObjects;
  images?: Record<string, AlbumImageEntry[]>;
  nodes?: {
    track?: TrackItem[];
    tracks?: TrackItem[];
    [key: string]: TrackItem[] | undefined;
  };
}

export interface TrackPhysicsInfo {
  duration_formatted?: string;
  duration_milliseconds?: number | string;
  sample_rate?: number;
  bits_per_sample?: number;
  bitrate_kbps?: number;
  encoding?: string;
  channels?: number;
}

export interface TrackItem {
  id?: string | number;
  tags?: TrackTags;
  objects?: TrackObjects;
  location?: NodeLocation;
  nodes?: { [key: string]: TrackItem[] | undefined };
}

/** The literal lock JSON returned by `/api/media/album/{id}`. */
export interface FullAlbum {
  album: AlbumLockData;
}

export interface AlbumCardItem {
  id: string;
  title?: string;
  artist?: string;
  images?: Record<string, AlbumImageEntry[]>;
  total_discs?: number;
  total_tracks?: number;
  duration_formatted?: string;
  virtual?: boolean;
  tags?: AlbumTags;
  colors?: AlbumColors | null;
}

export interface AlbumDictEntry {
  id: string;
  tags?: AlbumTags;
  objects?: AlbumObjects;
  images?: Record<string, AlbumImageEntry[]>;
}

export interface GrouperTreeNode {
  value: string;
  label: string;
  sublabel?: string | null;
  sort?: string | number;
  count?: number;
  pct?: number;
  duration_millis?: number;
  total_tracks?: number;
  total_discs?: number;
  parent?: string | null;
  children?: GrouperTreeNode[];
}

export interface QueueItem {
  id: number | string;
  file: string;
  title?: string | null;
  artist?: string | null;
  media_type?: string | null;
  media_id?: string | null;
  node_index?: number | null;
  track_no?: number | string | null;
  disc_no?: number | null;
  duration?: string | null;
  duration_ms?: number | null;
}

export interface MappedQueueTrack {
  id: string | number;
  file: string;
  isPlaying: boolean;
  trackNo: string | number;
  discNo: number;
  duration: string;
  durationMs: number;
  title: string;
  artist: string;
  albumId: string | null;
}

export interface QueueGroup {
  albumId: string | null;
  albumMeta: AlbumDictEntry | null;
  tracks: MappedQueueTrack[];
}

export interface FilterDef {
  label: string;
}

export interface GrouperDef {
  label: string;
  index?: boolean;
  count?: boolean;
  reverse?: boolean;
}

export interface OrderDef {
  label: string;
  reverse?: boolean;
}

export interface LibraryDef {
  label: string;
  filters?: string[];
  groupers?: string[];
  orders?: string[];
  allowed_filters?: string[];
  allowed_groupers?: string[];
  allowed_orders?: string[];
}

export interface ShelfDef {
  label: string;
  reverse?: boolean;
}

export interface CabinetDef {
  label: string;
  shelves?: string[];
  orders?: string[];
  allowed_shelves?: string[];
  allowed_orders?: string[];
}

export interface LogicManifest {
  filters?: Record<string, FilterDef>;
  libraries?: Record<string, LibraryDef>;
  groupers?: Record<string, GrouperDef>;
  orders?: Record<string, OrderDef>;
  shelves?: Record<string, ShelfDef>;
  cabinets?: Record<string, CabinetDef>;
  filters_order?: string[];
  libraries_order?: string[];
  groupers_order?: string[];
  orders_order?: string[];
  shelves_order?: string[];
  cabinets_order?: string[];
}

export interface FilterState {
  key: string | null;
  val: string | null;
}

export interface SortState {
  key: string | null;
  order: string;
}

export interface LibraryState {
  activeLibraryFilter?: string | null;
  activeFilter?: FilterState;
  userSortPreference?: string | null;
  userSortOrder?: string;
  activeSidebarGrouper?: string | null;
  activeSort?: SortState;
}

export interface PersistedState {
  activeTab?: string;
  homeSubView?: "libraries" | "cabinets";
  librariesState?: Record<string, LibraryState>;
  activeLibrary?: string;
  activeCabinet?: string;
  activeLibraryFilter?: string | null;
  sortKey?: string | null;
  sortOrder?: string;
  groupKey?: string | null;
  filter?: FilterState;
  activeShelf?: string | null;
  activeShelfOrder?: string;
  activeShelfOrderReverse?: boolean;
  isShaderEnabled?: boolean;
  sidebarWidth?: number;
}

export interface DropdownItem {
  key: string;
  label: string;
}

export type LibraryListItem = LibraryDef & { key: string };
export type ShelfListItem = ShelfDef & { key: string };
export type CabinetListItem = CabinetDef & { key: string };

export interface ActionDef {
  label: string;
  description?: string | null;
}

export interface MpdStatusPayload {
  state: string;
  media_type?: string | null;
  media_id?: string | null;
  node_index?: number | null;
  file?: string | null;
  title?: string | null;
  artist?: string | null;
  elapsed?: number | string | null;
  duration?: number | string | null;
  queue?: QueueItem[];
}

export type WsMessage =
  | {
      type: "INIT";
      media?: Record<string, Record<string, AlbumDictEntry>>;
      manifest?: LogicManifest;
      actions?: Record<string, ActionDef>;
      shelves?: Record<string, string[]>;
      ui_state?: PersistedState;
    }
  | { type: "VIEW_DATA"; ids?: string[] }
  | { type: "SHELF_DATA"; ids?: string[] }
  | { type: "GROUP_RESULT"; key: string; result: GrouperTreeNode[] }
  | {
      type: "MEDIA_UPDATED";
      media_type: string;
      removed?: string[];
      updated?: Record<string, AlbumDictEntry>;
      shelves?: Record<string, string[]>;
    }
  | { type: "LOGIC_UPDATE"; manifest?: LogicManifest }
  | (MpdStatusPayload & { type: "MPD_STATUS" });
