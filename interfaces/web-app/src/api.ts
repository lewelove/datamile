import type { FullAlbum } from "./types.ts";

export function getSocketUrl(): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const host = import.meta.env.VITE_DATAMILE_HOST || "127.0.0.1:8000";
  return `${protocol}//${host}/ws`;
}

export async function fetchMedia(
  type: string,
  id: string,
  depth?: number
): Promise<FullAlbum | null> {
  const encodedId = encodeURIComponent(id);
  const suffix = depth !== undefined ? `?depth=${depth}` : "";
  const response = await fetch(`/api/media/${type}/${encodedId}${suffix}`);
  if (!response.ok) {
    return null;
  }
  return (await response.json()) as FullAlbum;
}

export async function playMedia(
  type: string,
  id: string,
  opts: { start?: number; count?: number; offset?: number } = {}
): Promise<unknown> {
  const encodedId = encodeURIComponent(id);
  const params = new URLSearchParams();
  if (opts.start !== undefined) {
    params.set("start", String(opts.start));
  }
  if (opts.count !== undefined) {
    params.set("count", String(opts.count));
  }
  if (opts.offset !== undefined) {
    params.set("offset", String(opts.offset));
  }
  const query = params.toString();
  const suffix = query ? `?${query}` : "";
  const response = await fetch(`/api/media/${type}/play/${encodedId}${suffix}`, {
    method: "POST"
  });
  return await response.json();
}

export async function queueMedia(
  type: string,
  id: string,
  opts: { start?: number; count?: number } = {}
): Promise<unknown> {
  const encodedId = encodeURIComponent(id);
  const params = new URLSearchParams();
  if (opts.start !== undefined) {
    params.set("start", String(opts.start));
  }
  if (opts.count !== undefined) {
    params.set("count", String(opts.count));
  }
  const query = params.toString();
  const suffix = query ? `?${query}` : "";
  const response = await fetch(`/api/media/${type}/queue/${encodedId}${suffix}`, {
    method: "POST"
  });
  return await response.json();
}

export async function jumpToQueueIndex(index: string | number): Promise<unknown> {
  const response = await fetch(`/api/playback/jump/${index}`, { method: "POST" });
  return await response.json();
}

export async function executeAction(
  name: string,
  id: string,
  extraParams: Record<string, string> = {}
): Promise<unknown> {
  const params = new URLSearchParams({ type: "album", id, ...extraParams });
  const response = await fetch(`/api/actions/${name}?${params.toString()}`, { method: "POST" });
  return await response.json();
}

export async function updateMedia(type: string, id: string): Promise<unknown> {
  const encodedId = encodeURIComponent(id);
  const response = await fetch(`/api/media/${type}/update/${encodedId}`, { method: "POST" });
  return await response.json();
}
