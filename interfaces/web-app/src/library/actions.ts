import type { ActionMap, ActionPresentation } from "../types.ts";

/** The only media type this app renders, so every scoped control targets it. */
export const MEDIA_ALBUM = "album";

/**
 * A generic action has no `media` map, so it applies to every media type.
 * A scoped action applies only when the caller passes a matching media type.
 */
export function hasAction(actions: ActionMap, name: string, mediaType: string | null): boolean {
  const entry = actions[name];
  if (!entry) {
    return false;
  }
  if (!entry.media) {
    return true;
  }
  return mediaType !== null && Object.hasOwn(entry.media, mediaType);
}

/**
 * Returns the label/description the client should show for this media type.
 * Generic actions keep their flat pair; scoped actions look up their per-media
 * presentation and return nothing when the media type is not declared.
 */
export function actionPresentation(
  actions: ActionMap,
  name: string,
  mediaType: string | null
): ActionPresentation | null {
  const entry = actions[name];
  if (!entry) {
    return null;
  }
  if (!entry.media) {
    return { label: entry.label, description: entry.description };
  }
  if (mediaType === null) {
    return null;
  }
  return entry.media[mediaType] ?? null;
}
