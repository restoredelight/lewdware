export type TaskTone = 'progress' | 'success' | 'warning' | 'error';

export interface TaskMessage {
	id: string;
	tone: TaskTone;
	message: string;
	sequence: number;
}

const tonePriority: Record<TaskTone, number> = {
	error: 400,
	warning: 300,
	progress: 200,
	success: 100
};

class TaskFeedback {
	entries = $state<TaskMessage[]>([]);
	private timers = new Map<string, ReturnType<typeof setTimeout>>();
	private sequence = 0;

	active = $derived.by(
		() =>
			[...this.entries].sort(
				(a, b) => tonePriority[b.tone] - tonePriority[a.tone] || a.sequence - b.sequence
			)[0] ?? null
	);
	queuedCount = $derived(Math.max(0, this.entries.length - 1));

	private set(id: string, tone: TaskTone, message: string) {
		this.clearTimer(id);
		const existing = this.entries.find((entry) => entry.id === id);
		if (existing) {
			existing.tone = tone;
			existing.message = message;
		} else {
			this.entries.push({ id, tone, message, sequence: this.sequence++ });
		}
		queueMicrotask(() => this.reconcileSuccessTimer());
	}

	progress(id: string, message: string) {
		this.set(id, 'progress', message);
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
		for (const id of [...this.timers.keys()]) {
			if (id !== active?.id) this.clearTimer(id);
		}
		if (active?.tone === 'success' && !this.timers.has(active.id)) {
			this.timers.set(
				active.id,
				setTimeout(() => this.dismiss(active.id), 2200)
			);
		}
	}
}

export const taskFeedback = new TaskFeedback();
