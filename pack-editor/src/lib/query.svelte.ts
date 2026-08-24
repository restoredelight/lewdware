/**
 * Fetching what a view renders, and refetching it when something changes it.
 *
 * The editor used to hold the whole behaviour document in `store.behaviour`, mutate it locally, and
 * describe each change to the backend as a JSON patch. The backend is now the only place the
 * document exists: a surface asks for what it renders when it renders, and asks again when an edit
 * or an event says the answer has moved. See `design/editor-data-flow.md`.
 *
 * Three things this has to get right, all of them learned from `ensureBehaviour`, the single-key
 * loader it generalizes:
 *
 * - **Coalesce by key.** Two surfaces mounting in the same tick must produce one request, and the
 *   slower reply must not overwrite what the first already established.
 * - **Guard by pack.** A reply for the pack that was open when the request went out is worthless
 *   once a different pack is open, and applying it would show one pack's data under another's name.
 * - **Report failures once.** Every load site used to hand-roll its own `loaded`/`loadError` pair
 *   and its own `catch`; that boilerplate lives here now.
 *
 * Queries are cached per key and shared between subscribers, so the Content tab's badge counts and
 * the editor it opens read one fetch, not two.
 */
import { untrack } from 'svelte';
import { store } from './store.svelte.js';

/** A cached query's shared state, independent of how many views are reading it. */
class Entry<T> {
	value = $state.raw<T | undefined>(undefined);
	error = $state<string | null>(null);
	/** Whether a request is in flight *and* we have nothing to show yet. */
	loading = $state(false);
	/** The request in flight, so a second subscriber waits rather than issuing its own. */
	inFlight: Promise<void> | null = null;
	/**
	 * The whole of the work started by the running request, including any follow-up pass.
	 *
	 * `inFlight` is cleared as soon as its own reply lands; callers that need to know the answer is
	 * *current* — a mutation waiting to scroll to what it added — have to wait for this instead.
	 */
	settled: Promise<void> = Promise.resolve();
	/** Whether something asked for a fresh answer while a request was already on its way. */
	refetchWanted = false;
	/** How many live views are reading this. */
	subscribers = 0;
	/** The pack the cached value belongs to. */
	packId: string | null = null;
	/**
	 * Whether the answer is known to have moved and has not been refetched.
	 *
	 * Set by {@link invalidate} on an entry that has never been read, so the first view to ask for
	 * it fetches rather than inheriting a placeholder.
	 */
	stale = false;

	constructor(
		readonly key: string,
		readonly fetcher: () => Promise<T>
	) {}

	/**
	 * Fetches, unless a request is already out — in which case that one's answer will do.
	 *
	 * For a view that wants the data. Compare {@link refresh}, for when the data has *moved*.
	 */
	async load(): Promise<void> {
		if (this.inFlight) return this.inFlight;
		return this.fetch();
	}

	/**
	 * Fetches an answer that reflects everything up to now.
	 *
	 * The difference from {@link load} is what happens when a request is already out: that request
	 * took its snapshot of the backend *before* whatever prompted this call, so its answer is
	 * already known to be out of date. Waiting for it and stopping there would leave the pre-edit
	 * answer as the cache's current value with nothing left to correct it. So it is followed.
	 */
	async refresh(): Promise<void> {
		if (this.inFlight) {
			this.refetchWanted = true;
			return this.inFlight.then(() => this.settled);
		}
		return this.fetch();
	}

	private async fetch(): Promise<void> {
		const packId = store.packId;
		this.loading = this.value === undefined;
		this.error = null;
		this.stale = false;
		// Assigned before the body can run: an `async` function body starts executing synchronously
		// up to its first `await`, so a fetcher that throws *synchronously* would otherwise reach
		// the `finally` before this assignment and leave `inFlight` pointing at a settled promise —
		// after which every later load would return early and the view would never update again.
		const request = (async () => {
			await Promise.resolve();
			try {
				const value = await this.fetcher();
				// A reply for a pack that is no longer open describes something the author has
				// closed. Adopting it would show the previous pack's content under this one's name.
				if (store.packId !== packId) return;
				this.value = value;
				this.packId = packId;
				this.error = null;
			} catch (error) {
				if (store.packId !== packId) return;
				this.error = String(error);
			} finally {
				this.loading = false;
				this.inFlight = null;
			}
			// Something asked for a fresh answer while this one was on its way, so this one is
			// already out of date however it turned out.
			if (this.refetchWanted) {
				this.refetchWanted = false;
				await this.fetch();
			}
		})();
		this.inFlight = request;
		this.settled = request;
		return request;
	}
}

const entries = new Map<string, Entry<unknown>>();

function entryFor<T>(key: string, fetcher: () => Promise<T>): Entry<T> {
	let entry = entries.get(key) as Entry<T> | undefined;
	if (!entry) {
		pruneOtherPacks();
		entry = new Entry(key, fetcher);
		entries.set(key, entry as Entry<unknown>);
	}
	return entry;
}

/**
 * Drops answers belonging to a pack that is no longer open.
 *
 * Done here rather than from the store's own pack-open path, which would make the store import
 * this module and this module import the store. Nothing that survives is *wrong* — a cached answer
 * records the pack it came from, and a view asking for one from a different pack refetches — but
 * without this the keys accumulate across every pack a session ever opens.
 */
function pruneOtherPacks(): void {
	const packId = store.packId;
	for (const [key, entry] of entries) {
		if (entry.subscribers === 0 && entry.packId !== null && entry.packId !== packId) {
			entries.delete(key);
		}
	}
}

/** What a view gets back from {@link query}. */
export interface Query<T> {
	/** The fetched value, or `undefined` before the first reply lands. */
	readonly current: T | undefined;
	/** The failure message, if the last attempt failed. */
	readonly error: string | null;
	/** True only while there is nothing to show yet — a refetch over existing data is silent. */
	readonly loading: boolean;
	/** Fetches again now. For a retry button, and for an edit that changed this view's data. */
	reload(): Promise<void>;
}

/**
 * Reads `key` from the backend, fetching it when this view mounts and again when it is invalidated.
 *
 * Call it during component initialization: it registers a subscriber for as long as the calling
 * component is alive, which is what tells {@link invalidate} whether anything still cares.
 *
 * ```svelte
 * const links = query('web_links', api.getWebLinks);
 * ```
 */
export function query<T>(key: string | (() => string), fetcher: () => Promise<T>): Query<T> {
	// The key is a function wherever it depends on something that changes — the file the inspector
	// is looking at, the pool a generic editor was given. Taking a plain string and capturing it
	// once would mean a view that switched targets kept showing the first one's answer.
	const keyOf = typeof key === 'function' ? key : () => key;
	let current = $derived(entryFor(keyOf(), fetcher));

	$effect(() => {
		const entry = current;
		entry.subscribers += 1;
		return () => {
			entry.subscribers -= 1;
		};
	});

	// Fetch when this view first wants the key, when the key changes, and when the pack does — a
	// key means "this pack's captions", not "the captions of whichever pack was open at mount".
	//
	// A cached answer is *kept* when the last view reading it goes away, and shown again to the
	// next one while a fresh copy is fetched behind it. That is what stops undo — which remounts
	// the whole tab — from blanking the surface to "Loading…" and taking the author's scroll
	// position with it.
	$effect(() => {
		const entry = current;
		const packId = store.packId;
		untrack(() => {
			if (entry.value === undefined || entry.packId !== packId || entry.stale) {
				void entry.load();
			}
		});
	});

	return {
		get current() {
			return current.value;
		},
		get error() {
			return current.error;
		},
		get loading() {
			return current.loading;
		},
		reload: () => current.load()
	};
}

/**
 * Marks every query whose key starts with `prefix` as out of date.
 *
 * Live queries refetch; ones nothing is reading are dropped, so the next view to ask gets a fresh
 * answer rather than one from before the edit. Called after a mutation lands, and from the events
 * that mean the backend changed something on its own (an import filling a slot, undo, discard).
 */
export function invalidate(prefix: string): Promise<void> {
	const refetching: Promise<void>[] = [];
	for (const [key, entry] of entries) {
		if (!key.startsWith(prefix)) continue;
		// Always reload one that has been read: the answer has moved, and a live view is showing
		// the old one. `subscribers` is not consulted, because a view between mounts still has a
		// cached answer that must not be handed to the next reader as if it were current.
		if (entry.subscribers > 0 || entry.value !== undefined) refetching.push(entry.refresh());
		else entry.stale = true;
	}
	// Awaited by callers that do something *after* the new answer is on screen — adding an entry
	// scrolls to it, and an entry that has not arrived yet cannot be scrolled to.
	return Promise.all(refetching).then(() => undefined);
}

/** Forgets every cached answer. For tests, and for anywhere the whole session restarts. */
export function resetQueries(): void {
	entries.clear();
}

/**
 * The keys the surfaces use, in one place so a mutation and the view it affects cannot disagree
 * about the spelling. Prefixes are meaningful: invalidating `behaviour` refetches everything under
 * it, which is what undo and discard want.
 */
export const keys = {
	summary: 'behaviour:summary',
	textPool: (kind: string) => `behaviour:pool:${kind}`,
	webLinks: 'behaviour:web_links',
	contentGroups: 'behaviour:content_groups',
	mediaSlots: 'behaviour:slots',
	timeline: 'behaviour:timeline',
	popupAttributes: (ids: number[]) => `behaviour:popup:${ids.join(',')}`,
	audioAttributes: (ids: number[]) => `behaviour:audio:${ids.join(',')}`,
	/** Everything the behaviour document backs — the prefix undo and discard invalidate. */
	behaviour: 'behaviour',
	tags: 'tags',
	artists: 'artists'
} as const;
