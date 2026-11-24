import type { AlbumLockData, TrackItem } from "../types.ts";

/**
 * Tracks live under `nodes.track` or `nodes.tracks`. Both names are supported
 * on purpose, so neither lock shape is a special case.
 */
export function tracksOf(album: AlbumLockData | null | undefined): TrackItem[] {
  return album?.nodes?.track ?? album?.nodes?.tracks ?? [];
}
