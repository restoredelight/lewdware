import { describe, expect, it } from "vitest";
import { duplicateStage, moveStage, normalizeTimeline, removeStage } from "./timelineModel.js";
import type { Stage, Timeline } from "./types.js";

const stage = (id: string): Stage => ({ id, label: id, content: {}, events: {} });
const ids = () => { let value = 0; return () => String(++value); };

describe("timeline editor operations", () => {
  it("keeps only the final stage unbounded", () => {
    const timeline: Timeline = { stages: [stage("a"), stage("b")], transitions: [] };
    normalizeTimeline(timeline, ids());
    expect(timeline.stages[0].end?.duration_seconds).toBe(300);
    expect(timeline.stages[1].end).toBeUndefined();
    expect(timeline.transitions).toHaveLength(1);
  });

  it("duplicates complete stage data with a fresh identity", () => {
    const source = stage("a"); source.events.popup = { interval: { kind: "fixed", seconds: 20 } };
    const timeline: Timeline = { stages: [source], transitions: [] };
    const copy = duplicateStage(timeline, 0, source, ids());
    expect(copy.id).not.toBe(source.id);
    expect(copy.events).toEqual(source.events);
    expect(timeline.stages[0].end).toBeDefined();
    expect(copy.end).toBeUndefined();
  });

  it("rebuilds only changed edges when stages move or are removed", () => {
    const timeline: Timeline = { stages: [stage("a"), stage("b"), stage("c")], transitions: [] };
    const createId = ids(); normalizeTimeline(timeline, createId);
    moveStage(timeline, 2, -1, createId);
    expect(timeline.stages.map((item) => item.id)).toEqual(["a", "c", "b"]);
    expect(timeline.transitions.map((item) => [item.from_stage, item.to_stage])).toEqual([["a", "c"], ["c", "b"]]);
    removeStage(timeline, timeline.stages[1], createId);
    expect(timeline.stages.map((item) => item.id)).toEqual(["a", "b"]);
  });
});
