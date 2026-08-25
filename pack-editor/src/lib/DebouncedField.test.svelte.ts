import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import DebouncedField from './DebouncedField.svelte';
import { cancelFields, flushFields } from './mutate.svelte.js';

// `mutate` is under test only for the bookkeeping it does *around* the commit; its history and
// feedback side effects belong to their own suites.
vi.mock('./history.svelte.js', () => ({ history: { record: () => {} } }));
vi.mock('$ui/taskFeedback.svelte.js', () => ({
	taskFeedback: { error: () => {}, dismiss: () => {} }
}));

/**
 * Mounts a field and hands back the pieces the tests drive it with.
 *
 * The snippet stands in for whatever widget a surface uses — the component renders one, it does not
 * *be* one, which is why the six editors can each keep their own input.
 */
function field(initial: string, oncommit: (value: string) => Promise<unknown>) {
	const target = document.createElement('div');
	document.body.appendChild(target);
	let shown = '';
	let set!: (next: string) => void;
	let commit!: () => void;

	// The snippet stands in for whatever widget a surface uses — the component renders one, it does
	// not *be* one, which is why the six editors can each keep their own input.
	const snippet = createRawSnippet<[string, (next: string) => void, () => void]>(
		(draft, setter, committer) => ({
			render: () => '<span></span>',
			setup: () => {
				$effect(() => {
					shown = draft();
					set = setter();
					commit = committer();
				});
			}
		})
	);

	const props = $state({
		value: initial,
		label: 'Edit thing',
		invalidates: [],
		oncommit,
		delay: 500,
		field: snippet
	});
	// The generic is not inferred through `mount`, which takes the component's props type as-is.
	const component = mount(DebouncedField<string>, { target, props });
	flushSync();
	return {
		get shown() {
			return shown;
		},
		type: (next: string) => {
			set(next);
			flushSync();
		},
		blur: () => {
			commit();
			flushSync();
		},
		setStored: (next: string) => {
			props.value = next;
			flushSync();
		},
		destroy: () => unmount(component)
	};
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
	vi.useFakeTimers({ shouldAdvanceTime: true });
	cancelFields();
});

describe('a field that holds what is being typed', () => {
	// The reason this exists: an input whose value only updates once the backend has answered loses
	// characters to a fast typist.
	it('shows what was typed rather than what is stored', async () => {
		const commit = vi.fn().mockResolvedValue(undefined);
		const view = field('stored', commit);

		view.type('typed');
		expect(view.shown).toBe('typed');
		expect(commit).not.toHaveBeenCalled();
		view.destroy();
	});

	it('sends once the author pauses', async () => {
		const commit = vi.fn().mockResolvedValue(undefined);
		const view = field('stored', commit);

		view.type('typed');
		await vi.advanceTimersByTimeAsync(500);

		expect(commit).toHaveBeenCalledExactlyOnceWith('typed', 'Edit thing');
		view.destroy();
	});

	// Keystrokes inside one burst are one edit, not one command each.
	it('coalesces a burst of keystrokes into one send', async () => {
		const commit = vi.fn().mockResolvedValue(undefined);
		const view = field('', commit);

		view.type('t');
		await vi.advanceTimersByTimeAsync(100);
		view.type('ty');
		await vi.advanceTimersByTimeAsync(100);
		view.type('typed');
		await vi.advanceTimersByTimeAsync(500);

		expect(commit).toHaveBeenCalledExactlyOnceWith('typed', 'Edit thing');
		view.destroy();
	});

	it('sends immediately on blur rather than waiting out the pause', async () => {
		const commit = vi.fn().mockResolvedValue(undefined);
		const view = field('', commit);

		view.type('typed');
		view.blur();
		await settle();

		expect(commit).toHaveBeenCalledExactlyOnceWith('typed', 'Edit thing');
		view.destroy();
	});

	// The stored value is the truth whenever the author is not editing — that is what lets an undo,
	// or an edit made on another surface, reach a field they have left alone.
	it('adopts the stored value while nothing is pending', () => {
		const view = field('first', vi.fn().mockResolvedValue(undefined));

		view.setStored('second');

		expect(view.shown).toBe('second');
		view.destroy();
	});

	// The bug that bit three times in the previous design: a refetch landing mid-word must not
	// overwrite what is being typed.
	it('does not adopt the stored value over an unsent edit', () => {
		const view = field('first', vi.fn().mockResolvedValue(undefined));

		view.type('being typed');
		view.setStored('arrived from elsewhere');

		expect(view.shown).toBe('being typed');
		view.destroy();
	});

	// Saving and closing wait on this. Resolving despite a failure would write a pack missing the
	// value the author can see in front of them.
	it('surfaces a failed write through the shared flush', async () => {
		const view = field('', vi.fn().mockRejectedValue(new Error('disk on fire')));

		view.type('unsaveable');
		await expect(flushFields()).rejects.toThrow();
		view.destroy();
	});

	it('resolves the shared flush when the write succeeded', async () => {
		const view = field('', vi.fn().mockResolvedValue(undefined));

		view.type('fine');
		await expect(flushFields()).resolves.toBeUndefined();
		view.destroy();
	});

	// A revert replaces what the field is holding an edit against; sending it afterwards would
	// write it back over the state the author asked to return to.
	it('drops the pending value when cancelled, and shows what is stored', async () => {
		const commit = vi.fn().mockResolvedValue(undefined);
		const view = field('stored', commit);

		view.type('abandoned');
		cancelFields();
		flushSync();
		await vi.advanceTimersByTimeAsync(500);

		expect(commit).not.toHaveBeenCalled();
		expect(view.shown).toBe('stored');
		view.destroy();
	});
});

describe('holding the draft across the round trip', () => {
	// The reported bug, and the one this component was built to make impossible. Letting go of the
	// draft when the write was merely *issued* leaves the field falling back to the last fetched
	// value for the length of the round trip — which is a field that visibly resets to what it said
	// before, and then loses the next keystroke to the value it reset to.
	it('keeps showing the typed value until the write has landed', async () => {
		let release!: () => void;
		const commit = vi.fn(() => new Promise<void>((resolve) => (release = resolve)));
		const view = field('stored', commit);

		view.type('typed');
		view.blur();
		await settle();
		// The refetch has not happened, so the stored value is still the old one.
		view.setStored('stored');
		expect(view.shown).toBe('typed');

		release();
		await settle();
		view.setStored('typed');
		expect(view.shown).toBe('typed');
		view.destroy();
	});

	// And a keystroke during the round trip must not be discarded by the older commit completing.
	it('keeps the newer text when the author types while a write is in flight', async () => {
		let release!: () => void;
		const commit = vi.fn(() => new Promise<void>((resolve) => (release = resolve)));
		const view = field('', commit);

		view.type('first');
		view.blur();
		await settle();

		view.type('first and more');
		release();
		await settle();
		// The stored value catches up with the *first* commit, which is already out of date.
		view.setStored('first');

		expect(view.shown).toBe('first and more');
		view.destroy();
	});

	// A failed write keeps the value on screen: the author can see it, and the flush that saving
	// waits on has to be able to stop a save that would write the pack without it.
	it('keeps the value and reports the failure when the write fails', async () => {
		const view = field('stored', vi.fn().mockRejectedValue(new Error('disk on fire')));

		view.type('typed');
		await expect(flushFields()).rejects.toThrow();
		view.setStored('stored');

		expect(view.shown).toBe('typed');
		view.destroy();
	});
});

describe('a field that goes away mid-edit', () => {
	// Leaving a tab sends what was typed, but the send outlives the component. Dropping it from the
	// barrier at that moment lets a save start while the write is still travelling — and lets a
	// failed one pass unnoticed, writing the pack without the value the author entered.
	it('is still waited on by a save started after it unmounted', async () => {
		let release!: () => void;
		const commit = vi.fn(() => new Promise<void>((resolve) => (release = resolve)));
		const view = field('stored', commit);

		view.type('typed');
		view.destroy();
		await settle();

		let saved = false;
		const barrier = flushFields().then(() => (saved = true));
		await settle();
		expect(saved, 'the detached write is still in flight').toBe(false);

		release();
		await barrier;
		expect(commit).toHaveBeenCalledExactlyOnceWith('typed', 'Edit thing');
	});

	it('still reports its failure to a save started after it unmounted', async () => {
		const view = field('stored', vi.fn().mockRejectedValue(new Error('disk on fire')));

		view.type('typed');
		view.destroy();

		await expect(flushFields()).rejects.toThrow();
	});
});

describe('the detached-write barrier', () => {
	// A write left behind by an unmounting field has to leave the barrier once it settles. Keeping
	// it makes every later save wait on something already finished — and keeping a *failed* one
	// makes every later save fail, for the rest of the session.
	it('stops waiting on a detached write once it has landed', async () => {
		const view = field('stored', vi.fn().mockResolvedValue(undefined));
		view.type('typed');
		view.destroy();
		await settle();

		await expect(flushFields()).resolves.toBeUndefined();
		// And again: a leaked entry would still be here.
		await expect(flushFields()).resolves.toBeUndefined();
	});

	it('does not fail every later save because one detached write failed', async () => {
		const failing = field('stored', vi.fn().mockRejectedValue(new Error('disk on fire')));
		failing.type('typed');
		failing.destroy();

		// A save that races the write still sees it fail — that is the point of the barrier.
		await expect(flushFields()).rejects.toThrow();

		// But the failure belongs to that write, not to the rest of the session: once it has
		// settled it leaves the barrier, and a later save is not blocked by it forever.
		await settle();
		await expect(flushFields()).resolves.toBeUndefined();
	});
});

describe('overlapping commits from one field', () => {
	// A commit's payload can be derived from what the query currently says — renaming a stage works
	// out the new name for the tag it owns from that tag's present name. Two sends overlapping would
	// both read the state from before either landed, and the second would ask to rename a tag the
	// first had already renamed: a silent no-op, leaving the stage's name and its tag disagreeing.
	it('does not start a second send while the first is still going', async () => {
		const order: string[] = [];
		let releaseFirst!: () => void;
		const commit = vi.fn((value: string) => {
			order.push(`start ${value}`);
			if (value === 'first') {
				return new Promise<void>((resolve) => {
					releaseFirst = () => {
						order.push('finish first');
						resolve();
					};
				});
			}
			order.push(`finish ${value}`);
			return Promise.resolve();
		});
		const view = field('', commit);

		view.type('first');
		view.blur();
		await settle();

		view.type('second');
		view.blur();
		await settle();
		expect(order, 'the second must not have started').toEqual(['start first']);

		releaseFirst();
		await settle();
		expect(order).toEqual(['start first', 'finish first', 'start second', 'finish second']);
		view.destroy();
	});
});

describe('cancelling while sends are queued', () => {
	// Clearing the draft does not reach a send already waiting its turn: it holds the value it was
	// handed. A discard while one send is in flight and another is queued would let the queued one
	// land after the pack had been restored — making it dirty again with an edit the author threw
	// away.
	it('does not run a queued send after the edit was thrown away', async () => {
		const sent: string[] = [];
		let releaseFirst!: () => void;
		const commit = vi.fn((value: string) => {
			sent.push(value);
			if (value === 'first') return new Promise<void>((resolve) => (releaseFirst = resolve));
			return Promise.resolve();
		});
		const view = field('stored', commit);

		view.type('first');
		view.blur();
		await settle();
		view.type('second');
		view.blur();
		await settle();
		expect(sent, 'the second is queued behind the first').toEqual(['first']);

		cancelFields();
		releaseFirst();
		await settle();

		expect(sent, 'the queued send belonged to the state being discarded').toEqual(['first']);
		view.destroy();
	});
});
