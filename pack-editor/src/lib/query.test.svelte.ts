import { beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import { invalidate, query, resetQueries } from './query.svelte.js';
import { store } from './store.svelte.js';

/**
 * Runs `body` in a reactive root, so the effects `query` registers actually run.
 *
 * Every test here is about what happens *after* a view is live — a refetch, a remount, a reply
 * arriving late — so a query whose effects never ran is not the thing under test.
 */
function mount<T>(body: () => T): { value: T; unmount: () => void } {
	let value!: T;
	const unmount = $effect.root(() => {
		value = body();
	});
	flushSync();
	return { value, unmount };
}

/** Lets pending fetches settle and the resulting updates land. */
async function settle() {
	await new Promise((resolve) => setTimeout(resolve, 0));
	flushSync();
}

beforeEach(() => {
	resetQueries();
	store.openPack('pack-id', 'Test', [], [], []);
});

describe('the query cache', () => {
	it('fetches once for two views reading the same key', async () => {
		const fetcher = vi.fn().mockResolvedValue('value');
		const a = mount(() => query('k', fetcher));
		const b = mount(() => query('k', fetcher));
		await settle();

		expect(fetcher).toHaveBeenCalledTimes(1);
		expect(a.value.current).toBe('value');
		expect(b.value.current).toBe('value');
		a.unmount();
		b.unmount();
	});

	// What a mutation depends on: the edit lands, the view is told its data moved, and the view
	// shows the new answer. Without this a rename writes to the database and the table carries on
	// showing the old name.
	it('refetches a live query when its key is invalidated', async () => {
		const fetcher = vi.fn().mockResolvedValueOnce('before').mockResolvedValueOnce('after');
		const view = mount(() => query('tags', fetcher));
		await settle();
		expect(view.value.current).toBe('before');

		await invalidate('tags');
		flushSync();

		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(view.value.current).toBe('after');
		view.unmount();
	});

	// `invalidate` is awaited by callers that act on the refreshed view — adding an entry scrolls
	// to it, and an entry that has not arrived cannot be scrolled to.
	it('resolves only once the refetched answer is in place', async () => {
		let release!: (value: string) => void;
		const fetcher = vi
			.fn()
			.mockResolvedValueOnce('before')
			.mockImplementationOnce(() => new Promise<string>((resolve) => (release = resolve)));
		const view = mount(() => query('k', fetcher));
		await settle();

		let done = false;
		const pending = invalidate('k').then(() => (done = true));
		await settle();
		expect(done, 'must not resolve while the refetch is in flight').toBe(false);

		release('after');
		await pending;
		flushSync();
		expect(view.value.current).toBe('after');
		view.unmount();
	});

	it('invalidates by prefix, so one edit reaches every pool', async () => {
		const captions = vi.fn().mockResolvedValueOnce('a').mockResolvedValueOnce('a2');
		const prompts = vi.fn().mockResolvedValueOnce('b').mockResolvedValueOnce('b2');
		const one = mount(() => query('behaviour:pool:caption', captions));
		const two = mount(() => query('behaviour:pool:prompt', prompts));
		await settle();

		await invalidate('behaviour');
		flushSync();

		expect(one.value.current).toBe('a2');
		expect(two.value.current).toBe('b2');
		one.unmount();
		two.unmount();
	});

	// Undo remounts the whole tab. If the cached answer went with the old subtree, every undo would
	// blank the surface to "Loading…" and take the author's scroll position with it.
	it('keeps showing its last answer across a remount', async () => {
		const fetcher = vi.fn().mockResolvedValue('value');
		const first = mount(() => query('k', fetcher));
		await settle();
		first.unmount();

		const second = mount(() => query('k', fetcher));
		expect(second.value.current).toBe('value');
		expect(second.value.loading).toBe(false);
		second.unmount();
	});

	// The other half of that: an answer kept past its last reader must not be handed to the next
	// one as if it were current, when an edit landed in between.
	it('refetches on remount when it was invalidated while nothing was reading it', async () => {
		const fetcher = vi.fn().mockResolvedValueOnce('before').mockResolvedValueOnce('after');
		const first = mount(() => query('k', fetcher));
		await settle();
		first.unmount();

		await invalidate('k');
		const second = mount(() => query('k', fetcher));
		await settle();

		expect(second.value.current).toBe('after');
		second.unmount();
	});

	it('reports a failure and leaves nothing showing', async () => {
		const fetcher = vi.fn().mockRejectedValue(new Error('nope'));
		const view = mount(() => query('k', fetcher));
		await settle();

		expect(view.value.error).toContain('nope');
		expect(view.value.current).toBeUndefined();
		view.unmount();
	});

	// A fetcher that throws synchronously used to leave `inFlight` pointing at a settled promise,
	// after which every later load returned early and the view never updated again.
	it('recovers from a fetcher that throws synchronously', async () => {
		const fetcher = vi
			.fn()
			.mockImplementationOnce(() => {
				throw new Error('sync boom');
			})
			.mockResolvedValueOnce('recovered');
		const view = mount(() => query('k', fetcher));
		await settle();
		expect(view.value.error).toContain('sync boom');

		await invalidate('k');
		flushSync();
		expect(view.value.current).toBe('recovered');
		view.unmount();
	});

	// A reply for a pack that is no longer open describes something the author has closed.
	it('discards a reply that arrives after the pack changed', async () => {
		let release!: (value: string) => void;
		const fetcher = vi.fn(() => new Promise<string>((resolve) => (release = resolve)));
		const view = mount(() => query('k', fetcher));
		// The fetcher is reached a microtask after the effect runs, so give it that much.
		await Promise.resolve();
		await Promise.resolve();

		store.openPack('other-pack', 'Other', [], [], []);
		release('stale');
		await settle();

		expect(view.value.current).toBeUndefined();
		view.unmount();
	});

	// A key means "this pack's captions", not "the captions of whichever pack was open at mount".
	it('refetches when the open pack changes', async () => {
		const fetcher = vi.fn().mockResolvedValueOnce('first').mockResolvedValueOnce('second');
		const view = mount(() => query('k', fetcher));
		await settle();
		expect(view.value.current).toBe('first');

		store.openPack('other-pack', 'Other', [], [], []);
		await settle();

		expect(view.value.current).toBe('second');
		view.unmount();
	});

	// The key is a function wherever it depends on something that changes — the file the inspector
	// is looking at. Capturing it once would keep showing the first target's answer.
	it('follows a key that changes', async () => {
		let id = $state(1);
		const fetcher = vi.fn(() => Promise.resolve(`file-${id}`));
		const view = mount(() => query(() => `media:${id}`, fetcher));
		await settle();
		expect(view.value.current).toBe('file-1');

		id = 2;
		await settle();

		expect(view.value.current).toBe('file-2');
		view.unmount();
	});
});

describe('invalidating a query that is already fetching', () => {
	// The request in flight took its snapshot of the backend before the edit landed. Handing back
	// its promise would let that pre-edit answer become the cache's current value with nothing left
	// to correct it — the surface would stay stale until some unrelated later invalidation.
	it('follows the reply in flight with a fresh pass', async () => {
		let release!: (value: string) => void;
		const fetcher = vi
			.fn()
			.mockImplementationOnce(() => new Promise<string>((resolve) => (release = resolve)))
			.mockResolvedValueOnce('after the edit');
		const view = mount(() => query('k', fetcher));
		await Promise.resolve();
		await Promise.resolve();

		// The edit lands while the first request is still out.
		const invalidated = invalidate('k');
		release('from before the edit');
		await invalidated;
		flushSync();

		expect(fetcher).toHaveBeenCalledTimes(2);
		expect(view.value.current).toBe('after the edit');
		view.unmount();
	});

	// And the caller waiting on it must not be told the view is current until the follow-up lands,
	// or `ContentList` scrolls to an entry the refetch has not produced yet.
	it('resolves the invalidation only after the follow-up settles', async () => {
		let releaseFirst!: (value: string) => void;
		let releaseSecond!: (value: string) => void;
		const fetcher = vi
			.fn()
			.mockImplementationOnce(() => new Promise<string>((resolve) => (releaseFirst = resolve)))
			.mockImplementationOnce(() => new Promise<string>((resolve) => (releaseSecond = resolve)));
		const view = mount(() => query('k', fetcher));
		await Promise.resolve();
		await Promise.resolve();

		let done = false;
		const invalidated = invalidate('k').then(() => (done = true));
		releaseFirst('stale');
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(done, 'the follow-up is still out').toBe(false);

		releaseSecond('fresh');
		await invalidated;
		flushSync();
		expect(view.value.current).toBe('fresh');
		view.unmount();
	});
});
