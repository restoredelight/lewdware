export type TaskTone = "progress" | "success" | "warning" | "error";

class TaskFeedback {
  tone = $state<TaskTone | null>(null);
  message = $state("");
  current = $state<number | null>(null);
  total = $state<number | null>(null);
  private timer: ReturnType<typeof setTimeout> | null = null;

  private set(tone: TaskTone, message: string, current: number | null = null, total: number | null = null) {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.tone = tone; this.message = message; this.current = current; this.total = total;
  }

  progress(message: string, current: number | null = null, total: number | null = null) { this.set("progress", message, current, total); }
  warning(message: string) { this.set("warning", message); }
  error(message: string) { this.set("error", message); }
  success(message: string) {
    this.set("success", message);
    this.timer = setTimeout(() => this.dismiss(), 2200);
  }
  dismiss() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null; this.tone = null; this.message = ""; this.current = null; this.total = null;
  }
}

export const taskFeedback = new TaskFeedback();
