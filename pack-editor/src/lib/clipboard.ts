import { taskFeedback } from "./taskFeedback.svelte.js";

export async function copyFileName(name: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(name);
    taskFeedback.success("clipboard", `Copied “${name}”`);
  } catch (error) {
    taskFeedback.error("clipboard", `Could not copy the file name: ${String(error)}`);
  }
}
