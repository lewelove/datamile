export const ROOT_TYPE = "album";
export const WEB_APP_NAMESPACE = "web-app";

export function getSocketUrl(): string {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const host = import.meta.env.VITE_DATAMILE_HOST || "127.0.0.1:8000";
  return `${protocol}//${host}/ws`;
}

async function request<T>(input: string, init?: RequestInit): Promise<T> {
  const response = await fetch(input, init);
  if (!response.ok) {
    throw new Error(`Request failed: ${response.status} ${input}`);
  }
  return (await response.json()) as T;
}

function post<T>(url: string, body: unknown = {}): Promise<T> {
  return request<T>(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body)
  });
}

export interface MediaTypeDef {
  dir: string;
  count: number;
  children: Record<string, string>;
}

export interface MediaDiscovery {
  types: Record<string, MediaTypeDef>;
  logic: Record<string, unknown>;
  actions: Record<string, { label?: string; description?: string }>;
  config: Record<string, unknown>;
}

export interface ItemsQuery extends Record<string, string | boolean | undefined> {
  library?: string;
  library_filter?: string;
  group?: string;
  group_val?: string;
  shelf?: string;
  sort?: string;
  reverse?: boolean;
  view?: string;
}

export interface ItemsResponse<TEntity> {
  ids: string[];
  items: Record<string, TEntity>[];
}

export interface PlaybackTarget {
  type: string;
  id: string;
  track_ids?: string[];
  offset?: number;
}

export interface ActionBody {
  type?: string;
  id?: string;
  ids?: string[];
  playing?: boolean;
  query?: string;
  options?: string;
}

function queryString(query: Record<string, string | boolean | undefined>): string {
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === "") {
      continue;
    }
    params.set(key, String(value));
  }
  const encoded = params.toString();
  return encoded ? `?${encoded}` : "";
}

export function fetchMedia(type?: string): Promise<MediaDiscovery> {
  return request<MediaDiscovery>(`/api/media${type ? `/${encodeURIComponent(type)}` : ""}`);
}

export function fetchItems<TEntity>(
  type: string,
  query: ItemsQuery
): Promise<ItemsResponse<TEntity>> {
  return request<ItemsResponse<TEntity>>(
    `/api/media/${encodeURIComponent(type)}/items${queryString(query)}`
  );
}

export function fetchItem<TEntity>(type: string, id: string): Promise<TEntity> {
  return request<TEntity>(
    `/api/media/${encodeURIComponent(type)}/item${queryString({ id })}`
  );
}

export function fetchGroups(
  type: string,
  grouper: string,
  query: { library?: string; library_filter?: string }
): Promise<unknown[]> {
  return request<unknown[]>(
    `/api/media/${encodeURIComponent(type)}/groups/${encodeURIComponent(grouper)}${queryString(query)}`
  );
}

export function fetchState(namespace: string): Promise<Record<string, unknown>> {
  return request<Record<string, unknown>>(`/api/state/${encodeURIComponent(namespace)}`);
}

export function saveState(
  namespace: string,
  patch: Record<string, unknown>
): Promise<unknown> {
  return post(`/api/state/${encodeURIComponent(namespace)}`, patch);
}

export function fetchPlayback(): Promise<unknown> {
  return request("/api/playback");
}

export function play(target: PlaybackTarget): Promise<unknown> {
  return post("/api/playback/play", target);
}

export function queueTracks(target: PlaybackTarget): Promise<unknown> {
  return post("/api/playback/queue", target);
}

export function jumpToQueueIndex(index: string | number): Promise<unknown> {
  return post("/api/playback/jump", { index });
}

export function playback(
  command: "next" | "prev" | "stop" | "clear" | "toggle-pause"
): Promise<unknown> {
  return post(`/api/playback/${command}`);
}

export function executeAction(name: string, body: ActionBody = {}): Promise<unknown> {
  return post(`/api/actions/${encodeURIComponent(name)}`, body);
}

export function updateEntity(id: string): Promise<unknown> {
  return executeAction("update", { type: ROOT_TYPE, id });
}
