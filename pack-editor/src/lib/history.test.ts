import { describe, expect, it, vi } from "vitest";
import { createHistory, type HistoryCommand } from "./history.svelte.js";

function setup(limits: { maxEntries?: number; maxStorageBytes?: number } = {}) {
  const savedStates: boolean[] = [];
  const markPackSaved = vi.fn();
  const disposeErrors: unknown[] = [];
  const history = createHistory({
    markHistoryChanged: (saved) => savedStates.push(saved),
    markPackSaved,
    onDisposeError: (error) => disposeErrors.push(error),
    ...limits,
  });
  return { history, savedStates, markPackSaved, disposeErrors };
}

function command(label: string, options: Partial<HistoryCommand> = {}): HistoryCommand {
  return {
    label,
    undo: vi.fn(async () => {}),
    redo: vi.fn(async () => {}),
    ...options,
  };
}

const nextTask = () => new Promise<void>((resolve) => queueMicrotask(() => resolve()));

describe("history", () => {
  it("records, undoes, and redoes commands in order", async () => {
    const { history } = setup();
    const first = command("First");
    const second = command("Second");
    history.record(first);
    history.record(second);
    expect(history.undoLabel).toBe("Second");
    expect(history.canRedo).toBe(false);

    await history.undo();
    expect(second.undo).toHaveBeenCalledOnce();
    expect(history.undoLabel).toBe("First");
    expect(history.redoLabel).toBe("Second");

    await history.redo();
    expect(second.redo).toHaveBeenCalledOnce();
    expect(history.cursor).toBe(2);
  });

  it("does not advance the cursor when undo or redo fails", async () => {
    const { history } = setup();
    history.record(command("Broken undo", { undo: vi.fn(async () => { throw new Error("undo"); }) }));
    await expect(history.undo()).rejects.toThrow("undo");
    expect(history.cursor).toBe(1);

    const working = command("Working");
    history.reset(true);
    history.record(working);
    await history.undo();
    working.redo = vi.fn(async () => { throw new Error("redo"); });
    await expect(history.redo()).rejects.toThrow("redo");
    expect(history.cursor).toBe(0);
  });

  it("disposes the complete redo branch after a new edit", async () => {
    const { history } = setup();
    const disposeSecond = vi.fn(async () => {});
    const disposeThird = vi.fn(async () => {});
    history.record(command("First"));
    history.record(command("Second", { dispose: disposeSecond }));
    history.record(command("Third", { dispose: disposeThird }));
    await history.undo();
    await history.undo();
    history.record(command("Branch"));
    await nextTask();
    expect(disposeSecond).toHaveBeenCalledOnce();
    expect(disposeThird).toHaveBeenCalledOnce();
    expect(history.canRedo).toBe(false);
  });

  it("tracks and returns to the saved position", async () => {
    const { history, savedStates, markPackSaved } = setup();
    history.record(command("Before save"));
    history.markSaved();
    expect(markPackSaved).toHaveBeenCalledOnce();
    history.record(command("After save"));
    await history.undo();
    expect(savedStates.at(-1)).toBe(true);
    await history.undo();
    expect(savedStates.at(-1)).toBe(false);
  });

  it("reserves pending imports without blocking later operations", async () => {
    const { history } = setup();
    const token = history.reserve("Import still in progress");
    const later = command("Later edit");
    history.record(later);
    await history.undo();
    expect(later.undo).toHaveBeenCalledOnce();
    expect(history.canUndo).toBe(false);
    expect(history.undoLabel).toBe("Import still in progress");

    const imported = command("Import 3 media items");
    history.finalize(token, imported);
    expect(history.canUndo).toBe(true);
    await history.undo();
    expect(imported.undo).toHaveBeenCalledOnce();
  });

  it("removes an empty pending import and invalidates saves made during one", () => {
    const { history } = setup();
    const token = history.reserve("Importing");
    history.markSaved();
    expect(history.savedCursor).toBe(-1);
    history.finalize(token, null);
    expect(history.cursor).toBe(0);
    expect(history.canUndo).toBe(false);
  });

  it("keeps exactly the configured number of undo operations", async () => {
    const { history } = setup({ maxEntries: 3 });
    const disposals = Array.from({ length: 5 }, () => vi.fn(async () => {}));
    for (let index = 0; index < 5; index++) history.record(command(String(index), { dispose: disposals[index] }));
    await nextTask();
    expect(history.cursor).toBe(3);
    expect(history.undoLabel).toBe("4");
    expect(disposals[0]).toHaveBeenCalledOnce();
    expect(disposals[1]).toHaveBeenCalledOnce();
    expect(disposals[2]).not.toHaveBeenCalled();
  });

  it("uses only the undo prefix for storage limits and never trims usable redo", async () => {
    const { history } = setup({ maxEntries: 3, maxStorageBytes: 10 });
    const entries = [command("A", { storageBytes: 3 }), command("B", { storageBytes: 3 }), command("C", { storageBytes: 4 })];
    entries.forEach((entry) => history.record(entry));
    await history.undo();
    await history.undo();
    expect(history.cursor).toBe(1);
    expect(history.redoLabel).toBe("B");
    await history.redo();
    await history.redo();
    expect(history.cursor).toBe(3);
    expect(history.canRedo).toBe(false);
  });

  it("evicts the oldest applied media entry when the undo budget is exceeded", async () => {
    const { history } = setup({ maxStorageBytes: 5 });
    const disposeOldest = vi.fn(async () => {});
    history.record(command("Old", { storageBytes: 3, dispose: disposeOldest }));
    history.record(command("New", { storageBytes: 3 }));
    await nextTask();
    expect(history.cursor).toBe(1);
    expect(history.undoLabel).toBe("New");
    expect(disposeOldest).toHaveBeenCalledOnce();
  });

  it("reports disposal failures without corrupting the remaining history", async () => {
    const { history, disposeErrors } = setup({ maxEntries: 1 });
    history.record(command("Old", { dispose: async () => { throw new Error("cleanup"); } }));
    history.record(command("New"));
    await nextTask();
    expect(disposeErrors).toHaveLength(1);
    expect(history.undoLabel).toBe("New");
  });
});
