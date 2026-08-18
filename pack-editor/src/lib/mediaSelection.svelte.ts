/**
 * The selection gestures every media surface shares.
 *
 * The Popups/All-media grid and the Audio list are laid out nothing alike — one is a wrapped grid
 * of tiles, the other a flat list of rows — but *selecting* in them is identical: plain click
 * replaces, Ctrl/Cmd click toggles, Shift click extends from an anchor, Escape clears, Space
 * toggles the active item, Ctrl/Cmd+A takes everything visible, Delete asks to remove it. Each had
 * its own copy, and they had already drifted: one cleared the active item on Escape and the other
 * did not, and one's Ctrl+A missed a capitalised `A`.
 *
 * The anchor lives here because it is the one piece of selection state the store does not hold —
 * it is about the *gesture*, not the pack, and it belongs to whichever surface is on screen.
 *
 * What is *not* here is movement: which item is next depends on the layout, so each surface works
 * out the destination index itself and hands it to {@link MediaSelection.moveTo}.
 */
import { store } from './store.svelte.js';
import type { MediaFile } from './types.js';

export class MediaSelection {
	/** What a live region reads out after the selection changes. */
	announcement = $state('');
	/** Where the last unmodified click landed — the fixed end of a Shift-click range. */
	anchor = $state<number | null>(null);

	/** Singular noun for the announcements: "media item", "audio file". */
	constructor(private noun: string) {}

	announce() {
		const count = store.mediaTab.selectedIds.size;
		this.announcement = `${count || 'No'} ${this.noun}${count === 1 ? '' : 's'} selected`;
	}

	/** Applies a click's modifiers. The caller decides what else a click means on its surface. */
	click(id: number, event: MouseEvent) {
		if (event.shiftKey && this.anchor != null) store.selectRange(this.anchor, id);
		else if (event.ctrlKey || event.metaKey) store.toggleSelection(id);
		else store.selectSingle(id);
		if (!event.shiftKey) this.anchor = id;
		this.announce();
	}

	/** Deselects everything and forgets the anchor — Escape, and a click on empty space. */
	clear() {
		store.clearSelection();
		store.mediaTab.gridActiveId = null;
		this.anchor = null;
		this.announce();
	}

	/**
	 * Moves the active item to `index` in `list`, taking the selection with it.
	 *
	 * `extend` grows a range from the anchor (Shift); `preserveSelection` moves the active item
	 * without touching what is selected (Ctrl/Cmd), which is how you reach a file to Space-toggle
	 * into an existing selection.
	 */
	moveTo(list: MediaFile[], index: number, extend: boolean, preserveSelection: boolean) {
		const current = store.mediaTab.gridActiveId;
		const nextId = list[index].id;
		store.mediaTab.gridActiveId = nextId;
		if (extend) {
			this.anchor ??= current ?? nextId;
			store.selectRange(this.anchor, nextId);
		} else if (!preserveSelection) {
			store.selectSingle(nextId);
			this.anchor = nextId;
		}
		this.announce();
	}

	/**
	 * The keys both surfaces answer the same way.
	 *
	 * Returns whether it took the event; a surface handles its own navigation keys only when this
	 * says no.
	 */
	keydown(event: KeyboardEvent): boolean {
		const active = store.mediaTab.gridActiveId;

		if (event.key === 'Escape') {
			event.preventDefault();
			this.clear();
			return true;
		}
		if (event.key === ' ' && active != null) {
			event.preventDefault();
			store.toggleSelection(active);
			this.anchor ??= active;
			this.announce();
			return true;
		}
		if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
			event.preventDefault();
			store.selectAll();
			this.announce();
			return true;
		}
		if (event.key === 'Delete' && store.mediaTab.selectedIds.size > 0) {
			event.preventDefault();
			store.requestMediaRemoval();
			return true;
		}
		return false;
	}
}
