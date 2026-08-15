import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Behaviour } from './types.js';

const mocks = vi.hoisted(() => ({
	api: { setBehaviour: vi.fn(), getBehaviour: vi.fn() },
	store: {
		behaviour: null as Behaviour | null,
		packId: 'pack-1',
		markBackupComplete: vi.fn(),
		markBackupFailed: vi.fn(),
		markBackupPending: vi.fn()
	},
	history: { record: vi.fn() },
	feedback: { error: vi.fn(), dismiss: vi.fn() }
}));

vi.mock('./api.js', () => ({ api: mocks.api }));
vi.mock('./store.svelte.js', () => ({ store: mocks.store }));
vi.mock('./history.svelte.js', () => ({ history: mocks.history }));
vi.mock('./taskFeedback.svelte.js', () => ({ taskFeedback: mocks.feedback }));

const value = (number: number) => ({ testValue: number }) as unknown as Behaviour;

async function scheduler() {
	return import('./behaviourSave.svelte.js');
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.resetModules();
	vi.clearAllMocks();
	mocks.api.setBehaviour.mockResolvedValue(undefined);
	mocks.store.behaviour = value(0);
	mocks.store.packId = 'pack-1';
});

// The document is loaded lazily by whichever surface needs it first, and more than one can be
// first: the editor opens on a media tab that wants it for the inspector, and the author can click
// Content before the reply lands. See `ensureBehaviour`.
describe('loading the behaviour document', () => {
	/** A fetch the test decides when to answer, for pinning down what happens in between. */
	function deferredFetch() {
		let release!: (document: Behaviour) => void;
		mocks.api.getBehaviour.mockReturnValue(
			new Promise<Behaviour>((resolve) => {
				release = resolve;
			})
		);
		return (document: Behaviour) => release(document);
	}

	beforeEach(() => {
		mocks.store.behaviour = null;
	});

	it('serves callers that arrive together from a single fetch', async () => {
		const save = await scheduler();
		mocks.api.getBehaviour.mockResolvedValue(value(1));

		const both = await Promise.all([save.ensureBehaviour(), save.ensureBehaviour()]);

		expect(mocks.api.getBehaviour).toHaveBeenCalledOnce();
		expect(both).toEqual([value(1), value(1)]);
		expect(mocks.store.behaviour).toEqual(value(1));
	});

	it('does not fetch again once the document is loaded', async () => {
		const save = await scheduler();
		mocks.store.behaviour = value(3);

		expect(await save.ensureBehaviour()).toEqual(value(3));
		expect(mocks.api.getBehaviour).not.toHaveBeenCalled();
	});

	// The reason the guard exists: an undo, a discard or an import can replace the document while a
	// lazy load is still in flight, and the reply it was waiting for is by then the older one.
	it('yields to a document that landed while it was loading', async () => {
		const save = await scheduler();
		const release = deferredFetch();
		const pending = save.ensureBehaviour();

		mocks.store.behaviour = value(9);
		release(value(1));

		expect(await pending).toEqual(value(9));
		expect(mocks.store.behaviour).toEqual(value(9));
	});

	it('drops a reply for a pack that is no longer open', async () => {
		const save = await scheduler();
		const release = deferredFetch();
		const pending = save.ensureBehaviour();

		mocks.store.packId = 'pack-2';
		release(value(1));

		expect(await pending).toBeNull();
		expect(mocks.store.behaviour).toBeNull();
	});

	it('reports a failure and lets the next caller retry', async () => {
		const save = await scheduler();
		mocks.api.getBehaviour.mockRejectedValueOnce(new Error('offline'));

		expect(await save.ensureBehaviour()).toBeNull();
		expect(mocks.feedback.error).toHaveBeenCalledWith(
			'behaviour-load',
			expect.stringContaining('offline')
		);

		mocks.api.getBehaviour.mockResolvedValue(value(2));
		expect(await save.ensureBehaviour()).toEqual(value(2));
		expect(mocks.api.getBehaviour).toHaveBeenCalledTimes(2);
	});
});

describe('behaviour save scheduler', () => {
	it('debounces and coalesces edits into one snapshot command', async () => {
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);
		mocks.store.behaviour = value(1);
		save.scheduleBehaviourSave();
		await vi.advanceTimersByTimeAsync(300);
		mocks.store.behaviour = value(2);
		save.scheduleBehaviourSave();
		await vi.advanceTimersByTimeAsync(499);
		expect(mocks.api.setBehaviour).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);

		expect(mocks.api.setBehaviour).toHaveBeenCalledOnce();
		expect(mocks.api.setBehaviour).toHaveBeenCalledWith(value(2));
		expect(mocks.history.record).toHaveBeenCalledOnce();
		expect(mocks.store.markBackupPending).toHaveBeenCalledTimes(2);
		expect(mocks.store.markBackupComplete).toHaveBeenCalledWith('behaviour');
	});

	it('flushes immediately and waits for persistence', async () => {
		let release!: () => void;
		mocks.api.setBehaviour.mockReturnValue(
			new Promise<void>((resolve) => {
				release = resolve;
			})
		);
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);
		mocks.store.behaviour = value(1);
		save.scheduleBehaviourSave();
		const flushed = save.flushBehaviourSave();
		await vi.advanceTimersByTimeAsync(0);
		expect(mocks.api.setBehaviour).toHaveBeenCalledOnce();
		let finished = false;
		void flushed.then(() => {
			finished = true;
		});
		await Promise.resolve();
		expect(finished).toBe(false);
		release();
		await flushed;
		expect(finished).toBe(true);
	});

	it('records the persisted edit in backend history', async () => {
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);
		mocks.store.behaviour = value(7);
		save.scheduleBehaviourSave();
		await save.flushBehaviourSave();
		expect(mocks.history.record).toHaveBeenCalledWith({ label: 'Edit pack behaviour' });
		expect(mocks.api.setBehaviour).toHaveBeenCalledOnce();
	});

	// The bug this guards: filling a wallpaper slot changed the pack, but nothing told the
	// frontend, so `history.sync()` never ran and Save stayed disabled over real unsaved changes.
	it('records history when adopting a behaviour the backend produced', async () => {
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);

		save.adoptBehaviour(value(5), { label: 'Set wallpaper' });

		expect(mocks.store.behaviour).toEqual(value(5));
		expect(mocks.history.record).toHaveBeenCalledWith({ label: 'Set wallpaper' });
	});

	it("re-baselines an adopted behaviour so it is not re-recorded as the user's own edit", async () => {
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);

		// The backend changed the document; a later local edit must report only itself.
		save.adoptBehaviour(value(5), { label: 'Set wallpaper' });
		mocks.history.record.mockClear();
		save.scheduleBehaviourSave();
		await save.flushBehaviourSave();

		expect(mocks.api.setBehaviour).toHaveBeenCalledWith(value(5));
		expect(mocks.history.record).not.toHaveBeenCalled();
	});

	// The baseline lives in its own module precisely so `history` can reset it after undo/redo
	// re-fetches the document. Reverting a behaviour change must leave the saver diffing against
	// the reverted version, not the one from before the undo.
	it('treats a re-baselined document as persisted, wherever the reset came from', async () => {
		const save = await scheduler();
		const { initializeBehaviourHistory } = await import('./behaviourBaseline.svelte.js');
		save.initializeBehaviourHistory(mocks.store.behaviour!);

		// Stand in for undo: the backend hands back a different document, and `history` re-baselines.
		mocks.store.behaviour = value(9);
		initializeBehaviourHistory(mocks.store.behaviour);

		save.scheduleBehaviourSave();
		await save.flushBehaviourSave();

		expect(mocks.api.setBehaviour).toHaveBeenCalledWith(value(9));
		expect(mocks.history.record).not.toHaveBeenCalled();
	});

	// The Edgeware importer fills the wallpaper/splash slots after the frontend has already fetched
	// the document, and nothing else refreshes it -- the slots showed as empty until the pack was
	// reopened.
	describe('media slots filled by an import', () => {
		const withSlots = (content: Record<string, unknown>, experience: unknown = null) =>
			({ version: 1, content, experience }) as unknown as Behaviour;

		it('takes on slots the backend filled after the document was fetched', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots({});
			save.initializeBehaviourHistory(mocks.store.behaviour);

			save.applyFilledMediaSlots([
				{ slot: { kind: 'wallpaper' }, name: 'Wallpaper.jpg' },
				{ slot: { kind: 'splash' }, name: 'loading_splash.gif' }
			]);

			expect(mocks.store.behaviour.content).toEqual({
				wallpaper: 'Wallpaper.jpg',
				splash: 'loading_splash.gif'
			});
		});

		it('fills a stage wallpaper by stage id, ignoring an unknown stage', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots(
				{},
				{ timeline: { stages: [{ id: 'stage-2', content: {} }], transitions: [] } }
			);
			save.initializeBehaviourHistory(mocks.store.behaviour);

			save.applyFilledMediaSlots([
				{ slot: { kind: 'stage_wallpaper', stage: 'stage-2' }, name: 'level2.png' },
				{ slot: { kind: 'stage_wallpaper', stage: 'gone' }, name: 'orphan.png' }
			]);

			const stages = (
				mocks.store.behaviour as unknown as {
					experience: { timeline: { stages: { content: { wallpaper?: string } }[] } };
				}
			).experience.timeline.stages;
			expect(stages[0].content.wallpaper).toBe('level2.png');
			expect(stages).toHaveLength(1);
		});

		// Mirrors `Behaviour::fill_media_reference`'s own only-if-empty rule: a slot the author set
		// while the import was still running is their answer.
		it('never overwrites a slot the author set during the import', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots({ wallpaper: 'author-picked.png' });
			save.initializeBehaviourHistory(mocks.store.behaviour);

			save.applyFilledMediaSlots([{ slot: { kind: 'wallpaper' }, name: 'Wallpaper.jpg' }]);

			expect(mocks.store.behaviour.content).toEqual({ wallpaper: 'author-picked.png' });
		});

		// The backend has already written these, so a save would be pure noise -- and swapping the
		// whole document in (rather than patching) would discard edits made during the import.
		it('does not schedule a save of its own', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots({});
			save.initializeBehaviourHistory(mocks.store.behaviour);

			save.applyFilledMediaSlots([{ slot: { kind: 'splash' }, name: 'loading_splash.gif' }]);
			await vi.advanceTimersByTimeAsync(1000);

			expect(mocks.api.setBehaviour).not.toHaveBeenCalled();
			expect(mocks.store.markBackupPending).not.toHaveBeenCalled();
		});

		it('is a no-op when no tab has fetched the document yet', async () => {
			const save = await scheduler();
			mocks.store.behaviour = null;

			expect(() =>
				save.applyFilledMediaSlots([{ slot: { kind: 'wallpaper' }, name: 'Wallpaper.jpg' }])
			).not.toThrow();
			expect(mocks.store.behaviour).toBeNull();
		});
	});

	it('reports failures and allows a later save to recover the promise chain', async () => {
		mocks.api.setBehaviour.mockRejectedValueOnce(new Error('offline')).mockResolvedValue(undefined);
		const save = await scheduler();
		save.initializeBehaviourHistory(mocks.store.behaviour!);
		mocks.store.behaviour = value(1);
		save.scheduleBehaviourSave();
		await expect(save.flushBehaviourSave()).rejects.toThrow('offline');
		expect(mocks.store.markBackupFailed).toHaveBeenCalledWith('behaviour', expect.any(Error));
		expect(mocks.feedback.error).toHaveBeenCalledWith(
			'behaviour-backup',
			expect.stringContaining('offline')
		);

		mocks.store.behaviour = value(2);
		save.scheduleBehaviourSave();
		await expect(save.flushBehaviourSave()).resolves.toBeUndefined();
		expect(mocks.api.setBehaviour).toHaveBeenCalledTimes(2);
		expect(mocks.feedback.dismiss).toHaveBeenCalledWith('behaviour-backup');
	});
});
