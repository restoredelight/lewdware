import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Behaviour } from './types.js';

const mocks = vi.hoisted(() => ({
	api: { setBehaviour: vi.fn() },
	store: {
		behaviour: null as Behaviour | null,
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

	it('re-baselines an adopted behaviour so it is not re-recorded as the user\'s own edit', async () => {
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
