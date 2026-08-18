import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Behaviour } from './types.js';

const mocks = vi.hoisted(() => ({
	api: { editBehaviour: vi.fn(), getBehaviour: vi.fn() },
	store: {
		behaviour: null as Behaviour | null,
		packId: 'pack-1',
		markBackupComplete: vi.fn(),
		markBackupFailed: vi.fn(),
		markBackupPending: vi.fn(),
		removeFilesById: vi.fn(),
		retagEverywhere: vi.fn()
	},
	history: { record: vi.fn() },
	feedback: { error: vi.fn(), dismiss: vi.fn() }
}));

vi.mock('./api.js', () => ({ api: mocks.api }));
vi.mock('./store.svelte.js', () => ({ store: mocks.store }));
vi.mock('./history.svelte.js', () => ({ history: mocks.history }));
vi.mock('./taskFeedback.svelte.js', () => ({ taskFeedback: mocks.feedback }));

const value = (number: number) => ({ testValue: number }) as unknown as Behaviour;

/** A document with enough real shape for a path to address something in it. */
const document = (caption = 'first'): Behaviour =>
	({
		version: 3,
		content: {
			content_groups: [],
			captions: [{ text: caption, tags: [] }],
			prompts: [],
			notifications: [],
			subliminals: [],
			web_links: []
		},
		experience: null
	}) as unknown as Behaviour;

/** What `edit_behaviour` hands back. */
const edit = (behaviour: Behaviour, deleted_ids: number[] = []) => ({
	behaviour,
	deleted_ids,
	removed_tags: [],
	renamed_tags: []
});

const CAPTION = 'content.captions.0.text';

async function scheduler() {
	return import('./behaviourSave.svelte.js');
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.resetModules();
	vi.clearAllMocks();
	mocks.api.editBehaviour.mockResolvedValue(edit(document()));
	mocks.store.behaviour = document();
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

describe('editing the behaviour document', () => {
	/** Types `text` into the first caption the way a surface would: mutate, then declare the edit. */
	function typeCaption(save: Awaited<ReturnType<typeof scheduler>>, text: string) {
		mocks.store.behaviour!.content.captions[0].text = text;
		save.editBehaviourField(CAPTION, 'Edit caption');
	}

	it('coalesces repeated edits to one field into a single patch', async () => {
		const save = await scheduler();
		typeCaption(save, 'ty');
		await vi.advanceTimersByTimeAsync(300);
		typeCaption(save, 'typed');
		await vi.advanceTimersByTimeAsync(499);
		expect(mocks.api.editBehaviour).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);

		expect(mocks.api.editBehaviour).toHaveBeenCalledOnce();
		expect(mocks.api.editBehaviour).toHaveBeenCalledWith(
			[{ path: CAPTION, value: 'typed' }],
			'Edit caption',
			[],
			[]
		);
		expect(mocks.history.record).toHaveBeenCalledWith({ label: 'Edit caption' });
		expect(mocks.store.markBackupPending).toHaveBeenCalledTimes(2);
		expect(mocks.store.markBackupComplete).toHaveBeenCalledWith('behaviour');
	});

	// A batch is one undo entry, so it has to be one action: moving on to a different field under a
	// different label sends what came before rather than folding the two together.
	it('sends the open batch when the author moves on to another action', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');
		mocks.store.behaviour!.content.wallpaper = 7;
		save.editBehaviourField('content.wallpaper', 'Set wallpaper');

		await vi.advanceTimersByTimeAsync(0);
		expect(mocks.api.editBehaviour).toHaveBeenCalledOnce();
		expect(mocks.api.editBehaviour).toHaveBeenLastCalledWith(
			[{ path: CAPTION, value: 'typed' }],
			'Edit caption',
			[],
			[]
		);

		await vi.advanceTimersByTimeAsync(500);
		expect(mocks.api.editBehaviour).toHaveBeenLastCalledWith(
			[{ path: 'content.wallpaper', value: 7 }],
			'Set wallpaper',
			[],
			[]
		);
	});

	// A click is complete when it happens; waiting out a debounce would only leave a window in
	// which the action is on screen but not stored.
	it('sends a committed edit immediately', async () => {
		const save = await scheduler();
		mocks.store.behaviour!.content.captions.push({ text: '', tags: [] });
		save.commitBehaviourEdit('content.captions', 'Add caption');

		// No 500ms wait, only the microtask the write chain runs on.
		await vi.advanceTimersByTimeAsync(0);
		expect(mocks.api.editBehaviour).toHaveBeenCalledWith(
			[{ path: 'content.captions', value: mocks.store.behaviour!.content.captions }],
			'Add caption',
			[],
			[]
		);
	});

	// Removing a stage retires the wallpaper that existed only for it. Both halves ride in one
	// command so they are one transaction and one undo entry -- see `MediaPack::edit_behaviour`.
	it('sends the media an action retires, and drops what came back deleted', async () => {
		mocks.api.editBehaviour.mockResolvedValue(edit(document(), [12]));
		const save = await scheduler();

		save.commitBehaviourEdit('experience.timeline', 'Remove stage', [12]);
		await vi.advanceTimersByTimeAsync(0);

		expect(mocks.api.editBehaviour).toHaveBeenCalledWith(
			[{ path: 'experience.timeline', value: null }],
			'Remove stage',
			[12],
			[]
		);
		expect(mocks.store.removeFilesById).toHaveBeenCalledWith([12], true);
	});

	// The backend keeps a retired file that something else still points at, and says so by
	// returning no deleted ids. Dropping it from the grid anyway would hide a file the pack has.
	it('leaves the grid alone when a retired file was kept', async () => {
		const save = await scheduler();

		save.commitBehaviourEdit('experience.timeline', 'Remove stage', [12]);
		await vi.advanceTimersByTimeAsync(0);

		expect(mocks.store.removeFilesById).not.toHaveBeenCalled();
	});

	// An ordinary edit retires nothing: the list is empty rather than absent, so the backend's
	// scenery cleanup has nothing to consider.
	it('retires nothing for an edit that only changes a field', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');
		await vi.advanceTimersByTimeAsync(500);

		expect(mocks.api.editBehaviour).toHaveBeenCalledWith(expect.anything(), 'Edit caption', [], []);
	});

	it('flushes immediately and waits for the write to land', async () => {
		let release!: (result: ReturnType<typeof edit>) => void;
		mocks.api.editBehaviour.mockReturnValue(
			new Promise<ReturnType<typeof edit>>((resolve) => {
				release = resolve;
			})
		);
		const save = await scheduler();
		typeCaption(save, 'typed');
		const flushed = save.flushBehaviourSave();
		await vi.advanceTimersByTimeAsync(0);
		expect(mocks.api.editBehaviour).toHaveBeenCalledOnce();
		let finished = false;
		void flushed.then(() => {
			finished = true;
		});
		await Promise.resolve();
		expect(finished).toBe(false);
		release(edit(document()));
		await flushed;
		expect(finished).toBe(true);
	});

	it('drops a cancelled edit instead of sending it later', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');
		save.cancelBehaviourSave();
		await vi.advanceTimersByTimeAsync(1000);
		await save.flushBehaviourSave();

		expect(mocks.api.editBehaviour).not.toHaveBeenCalled();
	});

	// The bug this guards: filling a wallpaper slot changed the pack, but nothing told the
	// frontend, so `history.sync()` never ran and Save stayed disabled over real unsaved changes.
	it('records history when adopting a behaviour the backend produced', async () => {
		const save = await scheduler();

		save.adoptBehaviour(value(5), { label: 'Set wallpaper' });

		expect(mocks.store.behaviour).toEqual(value(5));
		expect(mocks.history.record).toHaveBeenCalledWith({ label: 'Set wallpaper' });
	});

	// The whole reason edits are patches. Every command that returns a document used to have to be
	// preceded by a flush, because a pending whole-document write held the pre-command version and
	// would land afterwards and undo it. Now the two compose: the command's result is taken on,
	// the edit still being typed is put back over it, and the patch that follows names only the
	// field it touched.
	it('adopts a backend document without losing an edit still being typed', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');

		const fromBackend = document();
		fromBackend.content.wallpaper = 7;
		save.adoptBehaviour(fromBackend, { label: 'Set wallpaper' });

		expect(mocks.store.behaviour!.content.wallpaper).toBe(7);
		expect(mocks.store.behaviour!.content.captions[0].text).toBe('typed');

		await vi.advanceTimersByTimeAsync(500);
		expect(mocks.api.editBehaviour).toHaveBeenCalledWith(
			[{ path: CAPTION, value: 'typed' }],
			'Edit caption',
			[],
			[]
		);
	});

	// The rest of the same window: the command can have read the document a moment before our
	// write reached the backend, so its reply comes back without an edit that is on its way.
	it('adopts a backend document without losing an edit already in flight', async () => {
		let release!: (result: ReturnType<typeof edit>) => void;
		mocks.api.editBehaviour.mockReturnValue(
			new Promise<ReturnType<typeof edit>>((resolve) => {
				release = resolve;
			})
		);
		const save = await scheduler();
		typeCaption(save, 'typed');
		await vi.advanceTimersByTimeAsync(500);
		expect(mocks.api.editBehaviour).toHaveBeenCalledOnce();

		// The reply the command read before our write landed.
		save.adoptBehaviour(document(), { label: 'Set wallpaper' });
		expect(mocks.store.behaviour!.content.captions[0].text).toBe('typed');

		release(edit(document('typed')));
		await vi.advanceTimersByTimeAsync(0);
	});

	it('stops re-applying an edit once its write has been acknowledged', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');
		await vi.advanceTimersByTimeAsync(500);

		// A later undo really did revert the caption; nothing local should put it back.
		save.adoptBehaviour(document('reverted'), { label: 'Undo' });
		expect(mocks.store.behaviour!.content.captions[0].text).toBe('reverted');
	});

	// The adopted document may no longer have a place for the edit -- the command whose result we
	// are taking on can be the one that deleted the stage it belonged to.
	it('drops a pending edit whose place the adopted document no longer has', async () => {
		const save = await scheduler();
		typeCaption(save, 'typed');

		const fromBackend = document();
		fromBackend.content.captions = [];
		expect(() => save.adoptBehaviour(fromBackend, { label: 'Undo' })).not.toThrow();
		expect(mocks.store.behaviour!.content.captions).toEqual([]);
	});

	it('reports failures and allows a later edit to recover the promise chain', async () => {
		mocks.api.editBehaviour
			.mockRejectedValueOnce(new Error('offline'))
			.mockResolvedValue(edit(document()));
		const save = await scheduler();
		typeCaption(save, 'one');
		await expect(save.flushBehaviourSave()).rejects.toThrow('offline');
		expect(mocks.store.markBackupFailed).toHaveBeenCalledWith('behaviour', expect.any(Error));
		expect(mocks.feedback.error).toHaveBeenCalledWith(
			'behaviour-backup',
			expect.stringContaining('offline')
		);

		typeCaption(save, 'two');
		await expect(save.flushBehaviourSave()).resolves.toBeUndefined();
		expect(mocks.api.editBehaviour).toHaveBeenCalledTimes(2);
		expect(mocks.feedback.dismiss).toHaveBeenCalledWith('behaviour-backup');
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

			save.applyFilledMediaSlots([
				{ slot: { kind: 'wallpaper' }, media_id: 1 },
				{ slot: { kind: 'splash' }, media_id: 2 }
			]);

			expect(mocks.store.behaviour.content).toEqual({ wallpaper: 1, splash: 2 });
		});

		it('fills a stage wallpaper by stage id, ignoring an unknown stage', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots(
				{},
				{ timeline: { stages: [{ id: 'stage-2', content: {} }], transitions: [] } }
			);

			save.applyFilledMediaSlots([
				{ slot: { kind: 'stage_wallpaper', stage: 'stage-2' }, media_id: 5 },
				{ slot: { kind: 'stage_wallpaper', stage: 'gone' }, media_id: 6 }
			]);

			const stages = (
				mocks.store.behaviour as unknown as {
					experience: { timeline: { stages: { content: { wallpaper?: number } }[] } };
				}
			).experience.timeline.stages;
			expect(stages[0].content.wallpaper).toBe(5);
			expect(stages).toHaveLength(1);
		});

		// Mirrors `Behaviour::fill_media_reference`'s own only-if-empty rule: a slot the author set
		// while the import was still running is their answer.
		it('never overwrites a slot the author set during the import', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots({ wallpaper: 42 });

			save.applyFilledMediaSlots([{ slot: { kind: 'wallpaper' }, media_id: 1 }]);

			expect(mocks.store.behaviour.content).toEqual({ wallpaper: 42 });
		});

		// The backend has already written these, so a save would be pure noise.
		it('does not schedule a save of its own', async () => {
			const save = await scheduler();
			mocks.store.behaviour = withSlots({});

			save.applyFilledMediaSlots([{ slot: { kind: 'splash' }, media_id: 2 }]);
			await vi.advanceTimersByTimeAsync(1000);

			expect(mocks.api.editBehaviour).not.toHaveBeenCalled();
			expect(mocks.store.markBackupPending).not.toHaveBeenCalled();
		});

		it('is a no-op when no tab has fetched the document yet', async () => {
			const save = await scheduler();
			mocks.store.behaviour = null;

			expect(() =>
				save.applyFilledMediaSlots([{ slot: { kind: 'wallpaper' }, media_id: 1 }])
			).not.toThrow();
			expect(mocks.store.behaviour).toBeNull();
		});
	});
});
