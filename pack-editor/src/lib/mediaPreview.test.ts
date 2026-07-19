import { beforeEach, describe, expect, it } from "vitest";
import { openMediaPreview } from "./mediaPreview.js";
import { store } from "./store.svelte.js";
import { taskFeedback } from "./taskFeedback.svelte.js";

describe("openMediaPreview", () => {
  beforeEach(() => {
    store.saveActive = false;
    store.saveBlocksPreviews = false;
    store.openedId = null;
    taskFeedback.dismiss("preview");
  });

  it("opens media when no save is active", () => {
    expect(openMediaPreview(42)).toBe(true);
    expect(store.openedId).toBe(42);
  });

  it("allows previews while a generation save is active", () => {
    store.saveActive = true;
    store.saveBlocksPreviews = false;

    expect(openMediaPreview(42)).toBe(true);
    expect(store.openedId).toBe(42);
  });

  it("leaves the current preview unchanged and warns during a save", () => {
    store.openedId = 7;
    store.saveActive = true;
    store.saveBlocksPreviews = true;

    expect(openMediaPreview(42)).toBe(false);
    expect(store.openedId).toBe(7);
    expect(taskFeedback.entries).toEqual(expect.arrayContaining([
      expect.objectContaining({
        id: "preview",
        tone: "warning",
        message: "Preview unavailable while the pack is being saved",
      }),
    ]));
  });
});
