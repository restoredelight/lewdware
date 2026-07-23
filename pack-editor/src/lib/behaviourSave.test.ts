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
