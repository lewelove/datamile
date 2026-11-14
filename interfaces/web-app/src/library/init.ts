import { sync } from "./sync.svelte.ts";
import { collection } from "./collection.svelte.ts";
import { view } from "./view.svelte.ts";
import { prewarmer } from "./prewarmer.svelte.ts";
import { updateConfig } from "../config.svelte.ts";
import { initColors } from "../colors.svelte.ts";
import { WEB_APP_NAMESPACE, fetchState } from "../api.ts";
import { applyPersistedState } from "./persistence.svelte.ts";
import type { PersistedState } from "../types.ts";
import appConfig from "../../config.json";

export async function initApp() {
  prewarmer.pruneTextures();

  initColors();
  sync.connect();

  collection.config = { ...collection.config, ...appConfig };
  updateConfig(appConfig);

  await collection.initialize();

  const persisted = (await fetchState(WEB_APP_NAMESPACE)) as PersistedState;
  if (Object.keys(persisted).length > 0) {
    applyPersistedState(persisted, view);
  }

  view.validateAndNormalizeLibrary();
  await view.refreshView(true);
  await view.refreshSidebar();
  prewarmer.orchestrate();
}
