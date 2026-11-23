import type { MpdStatusPayload, QueueItem } from "../types.ts";

export const player = $state<{
  state: string;
  currentMediaType: string | null;
  currentMediaId: string | null;
  currentNodeIndex: number | null;
  currentFile: string | null;
  title: string;
  artist: string;
  elapsed: number;
  duration: number;
  lastUpdated: number;
  queue: QueueItem[];
}>({
  state: "stop",
  currentMediaType: null,
  currentMediaId: null,
  currentNodeIndex: null,
  currentFile: null,
  title: "",
  artist: "",
  elapsed: 0,
  duration: 0,
  lastUpdated: 0,
  queue: []
});

export function updatePlayerState(data: MpdStatusPayload) {
  player.state = data.state;
  player.currentMediaType = data.media_type ?? null;
  player.currentMediaId = data.media_id ?? null;
  player.currentNodeIndex = data.node_index ?? null;
  player.currentFile = data.file ?? null;
  player.title = data.title || "";
  player.artist = data.artist || "";
  player.elapsed =
    typeof data.elapsed === "number" ? data.elapsed : parseFloat(String(data.elapsed || 0));
  player.duration =
    typeof data.duration === "number" ? data.duration : parseFloat(String(data.duration || 0));
  player.lastUpdated = performance.now();

  if (data.queue) {
    player.queue = data.queue;
  }
}
