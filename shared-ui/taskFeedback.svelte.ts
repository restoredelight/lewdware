/**
 * The one transient status message either app is showing, and the queue behind it.
 *
 * Shared because both apps want the same rules: an error outranks a warning outranks progress,
 * successful outcomes are silent (the result is already on screen), and only the single
 * highest-priority message is drawn — the rest are counted, not stacked.
 */
export type TaskTone = 'progress' | 'success' | 'warning' | 'error';

export interface TaskMessage {
	id: string;
	tone: TaskTone;
	message: string;
	current: number | null;
	total: number | null;
	sequence: number;
}

const tonePriority: Record<TaskTone, number> = {
	error: 400,
	warning: 300,
	progress: 200,
	success: 100
};
/**
 * Tie-breakers among messages of equal tone, by task id. Saving a pack outranks the history
 * entry it produces, which outranks a background upload — so a save's progress bar is not
 * displaced by the undo entry it creates. An id not listed here scores zero, which is why an
 * app that names none of these is ordered by tone and arrival alone.
 */
const taskPriority: Record<string, number> = { save: 40, history: 30, upload: 20 };

class TaskFeedback {
	entries = $state<TaskMessage[]>([]);
	private timers = new Map<string, ReturnType<typeof setTimeout>>();
	private sequence = 0;

	active = $derived.by(
		() =>
			[...this.entries].sort((a, b) => {
				const priority = (entry: TaskMessage) =>
					tonePriority[entry.tone] + (taskPriority[entry.id] ?? 0);
				return priority(b) - priority(a) || a.sequence - b.sequence;
			})[0] ?? null
	);
	queuedCount = $derived(Math.max(0, this.entries.length - 1));

	private set(
		id: string,
		tone: TaskTone,
		message: string,
		current: number | null = null,
		total: number | null = null
	) {
		this.clearTimer(id);
		const existing = this.entries.find((entry) => entry.id === id);
		if (existing) {
			existing.tone = tone;
			existing.message = message;
			existing.current = current;
			existing.total = total;
		} else {
			this.entries.push({ id, tone, message, current, total, sequence: this.sequence++ });
		}
		queueMicrotask(() => this.reconcileSuccessTimer());
	}

	progress(
		id: string,
		message: string,
		current: number | null = null,
		total: number | null = null
	) {
		this.set(id, 'progress', message, current, total);
	}
	warning(id: string, message: string) {
		this.set(id, 'warning', message);
	}
	error(id: string, message: string) {
		this.set(id, 'error', message);
	}
	// Successful outcomes are silent: the result is already visible in the UI.
	success(id: string, _message?: string) {
		this.dismiss(id);
	}
	// Brief visible confirmation, only for actions with no other visible effect (e.g. copying).
	confirm(id: string, message: string) {
		this.set(id, 'success', message);
	}
	dismiss(id = this.active?.id) {
		if (!id) return;
		this.clearTimer(id);
		this.entries = this.entries.filter((entry) => entry.id !== id);
		queueMicrotask(() => this.reconcileSuccessTimer());
	}
	private clearTimer(id: string) {
		const timer = this.timers.get(id);
		if (timer) clearTimeout(timer);
		this.timers.delete(id);
	}
	private reconcileSuccessTimer() {
		const active = this.active;
		for (const id of [...this.timers.keys()]) if (id !== active?.id) this.clearTimer(id);
		if (active?.tone === 'success' && !this.timers.has(active.id)) {
			this.timers.set(
				active.id,
				setTimeout(() => this.dismiss(active.id), 2200)
			);
		}
	}
}

export const taskFeedback = new TaskFeedback();
