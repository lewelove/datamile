import { collection } from "./collection.svelte.ts";

class Prewarmer {
  pinnedTextures: Map<string, ImageBitmap> = $state(new Map());
  private validUrls = new Set<string>();
  private inFlight = new Set<string>();
  private failedUrls = new Set<string>();
  private pendingUpdates = false;
  private lastFlush = Date.now();
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  /// Dict changed, so rebuild the valid set before warming new covers.
  refresh() {
    this.pruneTextures();
    this.orchestrate();
  }

  pruneTextures() {
    this.validUrls.clear();
    for (const album of Object.values(collection.dict)) {
      const url = collection.getThumbnailUrl(album);
      if (url) {
        this.validUrls.add(url);
      }
    }

    let evicted = false;
    this.pinnedTextures.forEach((bitmap, url) => {
      if (!this.validUrls.has(url)) {
        if (bitmap && typeof bitmap.close === "function") {
          bitmap.close();
        }
        this.pinnedTextures.delete(url);
        this.failedUrls.delete(url);
        evicted = true;
      }
    });

    if (evicted) {
      this.scheduleFlush();
    }
  }

  flush() {
    if (!this.pendingUpdates) {
      return;
    }
    this.pinnedTextures = new Map(this.pinnedTextures);
    this.pendingUpdates = false;
    this.lastFlush = Date.now();
    if (this.flushTimer) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
  }

  scheduleFlush() {
    this.pendingUpdates = true;
    if (Date.now() - this.lastFlush > 50) {
      this.flush();
    } else if (!this.flushTimer) {
      this.flushTimer = setTimeout(() => this.flush(), 50);
    }
  }

  async loadNow(url: string) {
    if (!url || this.pinnedTextures.has(url) || this.inFlight.has(url) || this.failedUrls.has(url)) {
      return;
    }

    this.inFlight.add(url);
    try {
      const res = await fetch(url);
      if (!res.ok) {
        throw new Error("Failed to load");
      }
      const blob = await res.blob();
      const bitmap = await createImageBitmap(blob, {
        premultiplyAlpha: "none",
        colorSpaceConversion: "default"
      });
      if (!this.validUrls.has(url)) {
        if (bitmap && typeof bitmap.close === "function") {
          bitmap.close();
        }
      } else {
        this.pinnedTextures.set(url, bitmap);
        this.scheduleFlush();
      }
    } catch {
      this.failedUrls.add(url);
    }
    this.inFlight.delete(url);
  }

  async orchestrate() {
    const concurrencyLimit = 6;
    const queue = Object.values(collection.dict);

    const processor = async () => {
      while (queue.length > 0) {
        const album = queue.shift();
        const url = collection.getThumbnailUrl(album);
        if (
          !url ||
          this.pinnedTextures.has(url) ||
          this.inFlight.has(url) ||
          this.failedUrls.has(url)
        ) {
          continue;
        }

        this.inFlight.add(url);
        try {
          const res = await fetch(url);
          if (!res.ok) {
            throw new Error("Failed to load");
          }
          const blob = await res.blob();
          const bitmap = await createImageBitmap(blob, {
            premultiplyAlpha: "none",
            colorSpaceConversion: "default"
          });
          if (!this.validUrls.has(url)) {
            if (bitmap && typeof bitmap.close === "function") {
              bitmap.close();
            }
          } else {
            this.pinnedTextures.set(url, bitmap);
            this.scheduleFlush();
          }
        } catch {
          this.failedUrls.add(url);
        }
        this.inFlight.delete(url);
      }
    };

    const workers = Array.from({ length: concurrencyLimit }, () => processor());
    await Promise.all(workers);
    this.flush();
  }
}

export const prewarmer = new Prewarmer();
