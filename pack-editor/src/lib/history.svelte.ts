import { store } from './store.svelte.js';
import { api } from './api.js';
import { fields } from './mutate.svelte.js';
import { invalidate, keys } from './query.svelte.js';
import type { HistoryStatus } from './types.js';

export interface HistoryRecord {
	label: string;
	storageBytes?: number;
}

class BackendHistory {
	canUndo = $state(false);
	canRedo = $state(false);
	undoLabel = $state<string | null>(null);
	redoLabel = $state<string | null>(null);
	busy = $state(false);
	private nextToken = 1;
	private refreshSequence = 0;

	private apply(status: HistoryStatus) {
		this.canUndo = status.can_undo;
		this.canRedo = status.can_redo;
		this.undoLabel = status.undo_label;
		this.redoLabel = status.redo_label;
		store.applyBackendHistoryState(status.at_saved_state);
	}

	async sync() {
		const sequence = ++this.refreshSequence;
		const status = await api.getHistoryStatus();
		if (sequence === this.refreshSequence) this.apply(status);
	}

	reset(_saved: boolean) {
		void this.sync();
	}
	record(_command: HistoryRecord) {
		void this.sync();
	}
	reserve(label: string) {
		const token = this.nextToken++;
		this.undoLabel = label;
		this.canUndo = false;
		return token;
	}
	touchPending(_token: number) {
		store.markHistoryChanged(false);
	}
	finalize(_token: number, _command: HistoryRecord | null) {
		void this.sync();
	}

	markSaved() {
		store.markPackSaved();
		void this.sync();
	}

	private async reloadEditor() {
		// Undo and redo flush first (so the edit being undone is a real entry to undo), which leaves
		// nothing pending by the time this runs. This covers the rest: a field still being typed
		// during the round trip belongs to the state being replaced, and sending it afterwards
		// would write it back over the one the author reverted to.
		fields.cancel();
		// A changeset can touch any table, so everything the surfaces are showing is suspect. This
		// is the one place a blanket invalidation is right, and it is cheap because only the open
		// tab's queries have subscribers.
		//
		// This replaced remounting the whole tab (`{#key store.historyRevision}` in `Editor`),
		// which existed back when a surface read a document that was swapped underneath it and had
		// no way to notice. Rebuilding the DOM threw away the author's scroll position on every
		// undo. Surfaces refetch now, and the two things the remount was really protecting are
		// handled where they belong: the media ids below, and `TimelineEditor`'s own fallback when
		// the stage it had selected is no longer in the timeline.
		invalidate(keys.behaviour);
		invalidate(keys.tags);
		invalidate(keys.artists);
		const [files, tags, artists, metadata] = await Promise.all([
			api.getFiles(),
			api.getAllTags(),
			api.getAllArtists(),
			api.getPackMetadata()
		]);
		store.files = files;
		store.allTags = tags;
		store.allArtists = artists;
		store.metadata = metadata;
		store.packName = metadata.name;
		store.reconcileSelection();
	}

	async undo() {
		if (this.busy || !this.canUndo) return;
		this.busy = true;
		this.canUndo = false;
		this.canRedo = false;
		++this.refreshSequence;
		try {
			const status = await api.undo();
			await this.reloadEditor();
			++this.refreshSequence;
			this.apply(status);
		} catch (error) {
			await this.sync().catch(() => {});
			throw error;
		} finally {
			this.busy = false;
		}
	}

	async redo() {
		if (this.busy || !this.canRedo) return;
		this.busy = true;
		this.canUndo = false;
		this.canRedo = false;
		++this.refreshSequence;
		try {
			const status = await api.redo();
			await this.reloadEditor();
			++this.refreshSequence;
			this.apply(status);
		} catch (error) {
			await this.sync().catch(() => {});
			throw error;
		} finally {
			this.busy = false;
		}
	}
}

export const history = new BackendHistory();
