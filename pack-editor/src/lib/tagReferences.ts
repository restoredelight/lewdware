import type { Behaviour } from "./types.js";

function lists(behaviour: Behaviour): { tags: string[]; area: "content" | "experience" }[] {
  const content = behaviour.content;
  const result: { tags: string[]; area: "content" | "experience" }[] = [
    ...content.content_groups.map((item) => ({ tags: item.tags, area: "content" as const })),
    ...content.captions.map((item) => ({ tags: item.tags, area: "content" as const })),
    ...content.prompts.map((item) => ({ tags: item.tags, area: "content" as const })),
    ...content.notifications.map((item) => ({ tags: item.tags, area: "content" as const })),
    ...content.subliminals.map((item) => ({ tags: item.tags, area: "content" as const })),
    ...content.web_links.map((item) => ({ tags: item.tags, area: "content" as const })),
    { tags: content.wallpaper_tags, area: "content" },
    { tags: content.splash_tags, area: "content" },
  ];
  for (const stage of behaviour.experience?.timeline.stages ?? []) {
    if (stage.content.tags) result.push({ tags: stage.content.tags, area: "experience" });
    if (stage.content.wallpaper_tags) result.push({ tags: stage.content.wallpaper_tags, area: "experience" });
  }
  return result;
}

export function tagUsage(behaviour: Behaviour, tag: string) {
  let content = 0, experience = 0;
  for (const list of lists(behaviour)) for (const value of list.tags) if (value === tag) list.area === "content" ? content++ : experience++;
  return { content, experience, total: content + experience };
}

export function behaviourTags(behaviour: Behaviour): string[] {
  return [...new Set(lists(behaviour).flatMap((list) => list.tags))];
}

export function rewriteTag(behaviour: Behaviour, from: string, to: string | null): Behaviour {
  // Callers unwrap reactive state at their framework boundary; this utility stays framework-
  // agnostic and keeps its input immutable.
  const copy = structuredClone(behaviour);
  for (const list of lists(copy)) {
    const rewritten = list.tags.flatMap((tag) => tag === from ? (to ? [to] : []) : [tag]);
    list.tags.splice(0, list.tags.length, ...new Set(rewritten));
  }
  return copy;
}
