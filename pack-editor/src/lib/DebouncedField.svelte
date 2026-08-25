<script lang="ts" generics="T">
	/**
	 * A field that holds what the author is typing, and sends it once they pause.
	 *
	 * This is the only client-side edit state left, and it exists for one reason: an input whose
	 * displayed value only updates once the backend has answered is a controlled input over an async
	 * boundary, and a fast typist loses characters to it. So the draft lives here and the field
	 * renders from it.
	 *
	 * It is deliberately *not* a correctness mechanism. Commands set one field each, so two edits to
	 * one entity commute and nothing needs accumulating — which means a mistake in here shows a
	 * stale value for a moment rather than losing a write. The previous design put both jobs in one
	 * global buffer and produced three bugs doing it: the draft's origin, its scope, and its
	 * lifetime. Here the lifetime is the component's, which is the part that kept going wrong.
	 *
	 * The stored value wins whenever the author is not editing, so an undo or an edit from another
	 * surface shows up immediately.
	 */
	import type { Snippet } from 'svelte';
	import { onDestroy, untrack } from 'svelte';
	import { mutate, registerField, trackDetached } from './mutate.svelte.js';

	type Props = {
		/** The stored value. Adopted whenever there is no unsent edit. */
		value: T;
		/** The author's word for this edit — it becomes the undo entry, and reaches `oncommit`. */
		label: string;
		/** Query key prefixes this edit changes, so the views showing them refetch. */
		invalidates: string[];
		/** Sends the value. Called on a pause, on blur, and on flush. */
		oncommit: (value: T, label: string) => Promise<unknown>;
		/** How long to wait after the last change. */
		delay?: number;
		/** Renders the widget: the value to show, a setter, and a commit-now for `onblur`. */
		field: Snippet<[T, (next: T) => void, () => void]>;
	};

	let { value, label, invalidates, oncommit, delay = 500, field }: Props = $props();

	// Seeded from the prop and then owned here. The effect below is what keeps it in step while the
	// author is not editing; reading `value` again during a keystroke is exactly what must not
	// happen, which is why this is a plain initialiser rather than a derived.
	let draft = $state<T>(untrack(() => value));
	let dirty = $state(false);
	let timer: ReturnType<typeof setTimeout> | null = null;
	/**
	 * Bumped by every keystroke, so a commit can tell whether the author has typed since it left.
	 *
	 * This is what decides when the draft may be let go of. Releasing it when the write was merely
	 * *issued* leaves the field falling back to the last fetched value for the length of the round
	 * trip — the field visibly resetting to what it said before.
	 */
	let generation = 0;

	// While nothing is pending, the stored value is the truth — that is what lets an undo, or an
	// edit made on another surface, reach a field the author has left alone.
	$effect(() => {
		const incoming = value;
		if (!dirty) draft = incoming;
	});

	function set(next: T) {
		draft = next;
		dirty = true;
		generation += 1;
		if (timer !== null) clearTimeout(timer);
		timer = setTimeout(() => void flush(), delay);
	}

	/** Sends the pending value now, if there is one. Rejects if the write fails. */
	export async function flush(): Promise<void> {
		if (timer !== null) {
			clearTimeout(timer);
			timer = null;
		}
		if (!dirty) return;
		const sending = draft;
		const mine = generation;
		// Routed through `mutate` rather than calling the command directly, so this edit records its
		// undo entry and — the part that matters here — the views showing it refetch. Without that
		// the query keeps the value from before the edit, and the resync below hands it straight
		// back to the field.
		const landed = await mutate(async () => void (await oncommit(sending, label)), {
			label,
			invalidates
		});
		if (!landed) {
			// Keep holding it: the author can still see what they typed, and `flushFields` has to be
			// able to stop a save that would write the pack without it.
			throw new Error(`Could not save ${label.toLowerCase()}.`);
		}
		// Let go only once the write *and* the refetch behind it have landed, and only if nothing
		// has been typed since this one went out.
		if (generation === mine) dirty = false;
	}

	/** Throws away the pending value — for an undo, whose result the field should adopt instead. */
	export function cancel() {
		if (timer !== null) clearTimeout(timer);
		timer = null;
		dirty = false;
		draft = value;
	}

	const unregister = registerField({ flush, cancel });
	onDestroy(() => {
		// Leaving the surface is not abandoning the edit: it is sent on the way out. The send
		// outlives this component, so it moves to the detached set rather than simply disappearing
		// — a save started a moment later still has to wait for it, and still has to see it fail.
		unregister();
		void trackDetached(flush()).catch(() => {});
	});
</script>

{@render field(draft, set, () => void flush().catch(() => {}))}
