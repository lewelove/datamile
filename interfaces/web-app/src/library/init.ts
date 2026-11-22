import { sync } from "./sync.svelte.ts";
import { prewarmer } from "./prewarmer.svelte.ts";
import { initColors } from "../colors.svelte.ts";

export function initApp() {
  prewarmer.pruneTextures();

  initColors();
  sync.connect();
}
