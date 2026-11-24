import type { AlbumLockData, TrackItem } from "../types.ts";

/**
 * Tracks live under `nodes.track`. Locks compiled before the rename still
 * carry `nodes.tracks`, so both names are read.
 */
export function tracksOf(album: AlbumLockData | null | undefined): TrackItem[] {
  return album?.nodes?.track ?? album?.nodes?.tracks ?? [];
}
