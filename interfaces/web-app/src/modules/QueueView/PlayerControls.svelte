<script lang="ts">
import { player } from "../player.svelte.ts";
import { playback } from "../../api.ts";

let isPlaying = $derived(player.state === "play");
let tickingElapsed = $state(0);
let duration = $derived(player.duration || 0);
let progress = $derived(duration > 0 ? (tickingElapsed / duration) * 100 : 0);

function formatTime(totalSeconds: number) {
  const s = Math.floor(totalSeconds || 0);
  const m = Math.floor(s / 60);
  const rs = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${m}:${pad(rs)}`;
}

$effect(() => {
  tickingElapsed = player.elapsed || 0;
  if (player.state !== "play") return;
  const startUpdated = player.lastUpdated;
  const startElapsed = player.elapsed;
  const currentDuration = player.duration || 0;
  const interval = setInterval(() => {
    const delta = (performance.now() - startUpdated) / 1000;
    tickingElapsed = Math.min(startElapsed + delta, currentDuration);
  }, 1000);
  return () => clearInterval(interval);
});

async function togglePlay() {
  try {
    await playback("toggle-pause");
  } catch {}
}

async function next() {
  try {
    await playback("next");
  } catch {}
}

async function prev() {
  try {
    await playback("prev");
  } catch {}
}
</script>

<footer class="player-controls">
  <div class="progress-container">
    <span class="time-display v-mono">{formatTime(tickingElapsed)}</span>
    <div class="progress-track">
      <div class="progress-fill" style:width="{progress}%"></div>
    </div>
    <span class="time-display v-mono">{formatTime(duration)}</span>
  </div>

  <div class="controls-container">
    <button type="button" class="v-btn-icon control-btn-lesser" onclick={prev} title="Previous">
      <span class="icon filled">skip_previous</span>
    </button>
    <button type="button" class="v-btn-icon control-btn" onclick={togglePlay} title="Toggle Play">
      <span class="icon filled control">{isPlaying ? "pause" : "play_arrow"}</span>
    </button>
    <button type="button" class="v-btn-icon control-btn-lesser" onclick={next} title="Next">
      <span class="icon filled">skip_next</span>
    </button>
  </div>
</footer>

<style>
.player-controls {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  padding-top: 16px;
  margin-top: 16px;
  gap: 16px;
}

.progress-container {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
}

.time-display {
  font-size: 12px;
  color: var(--text-subtle);
  user-select: none;
  width: 40px;
  text-align: center;
}

.progress-track {
  flex: 1;
  height: 4px;
  background-color: var(--border-muted);
  border-radius: 2px;
  position: relative;
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background-color: var(--text-muted);
  border-radius: 2px;
}

.controls-container {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
}

.control-btn {
  width: 36px;
  height: 36px;
  border-radius: 18px;
  font-size: 24px !important;
  flex-shrink: 0;
}

.control-btn-lesser {
  width: 32px;
  height: 32px;
  border-radius: 16px;
  flex-shrink: 0;
}
</style>
