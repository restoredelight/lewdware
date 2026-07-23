import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { MetadataDto } from './types.js';

const mocks = vi.hoisted(() => ({
	api: { setPackMetadata: vi.fn(), savePackMetadata: vi.fn() },
	store: {
		metadata: null as MetadataDto | null,
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

const metadata = (name: string): MetadataDto => ({
	name,
	creator: null,
	description: null,
	version: null,
	recommended_mode: null
});
const scheduler = () => import('./metadataSave.svelte.js');

beforeEach(() => {
	vi.useFakeTimers();
	vi.resetModules();
	vi.clearAllMocks();
	mocks.api.setPackMetadata.mockResolvedValue(undefined);
	mocks.api.savePackMetadata.mockResolvedValue(undefined);
	mocks.store.metadata = metadata('Initial');
});

describe('metadata save scheduler', () => {
	it('debounces edits and writes only the latest snapshot', async () => {
		const save = await scheduler();
		save.initializeMetadataHistory(metadata('Initial'));
		save.scheduleMetadataSave(metadata('First'));
		await vi.advanceTimersByTimeAsync(400);
		save.scheduleMetadataSave(metadata('Latest'));
		await vi.advanceTimersByTimeAsync(599);
		expect(mocks.api.setPackMetadata).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		expect(mocks.api.setPackMetadata).toHaveBeenCalledOnce();
		expect(mocks.api.setPackMetadata).toHaveBeenCalledWith(metadata('Latest'));
		expect(mocks.api.savePackMetadata).toHaveBeenCalledOnce();
		expect(mocks.history.record).toHaveBeenCalledOnce();
	});

	it('flushes pending metadata immediately', async () => {
		const save = await scheduler();
		save.initializeMetadataHistory(metadata('Initial'));
		save.scheduleMetadataSave(metadata('Flushed'));
		await save.flushMetadataSave();
		expect(mocks.api.setPackMetadata).toHaveBeenCalledWith(metadata('Flushed'));
		await vi.advanceTimersByTimeAsync(600);
		expect(mocks.api.setPackMetadata).toHaveBeenCalledOnce();
	});

	it('records the persisted edit in backend history', async () => {
		const save = await scheduler();
		save.initializeMetadataHistory(metadata('Initial'));
		save.scheduleMetadataSave(metadata('Changed'));
		await save.flushMetadataSave();
		expect(mocks.history.record).toHaveBeenCalledWith({ label: 'Edit pack metadata' });
		expect(mocks.api.setPackMetadata).toHaveBeenCalledOnce();
	});

	it('retains failed metadata and succeeds when flushed again', async () => {
		mocks.api.savePackMetadata
			.mockRejectedValueOnce(new Error('disk full'))
			.mockResolvedValue(undefined);
		const save = await scheduler();
		save.initializeMetadataHistory(metadata('Initial'));
		save.scheduleMetadataSave(metadata('Retry me'));
		await expect(save.flushMetadataSave()).rejects.toThrow('disk full');
		expect(mocks.store.markBackupFailed).toHaveBeenCalledWith('metadata', expect.any(Error));
		expect(mocks.feedback.error).toHaveBeenCalledWith(
			'metadata-backup',
			expect.stringContaining('disk full')
		);
		await expect(save.flushMetadataSave()).resolves.toBeUndefined();
		expect(mocks.api.setPackMetadata).toHaveBeenCalledTimes(2);
		expect(mocks.feedback.dismiss).toHaveBeenCalledWith('metadata-backup');
	});

	it('cancels a pending debounce without writing', async () => {
		const save = await scheduler();
		save.scheduleMetadataSave(metadata('Discarded'));
		save.cancelMetadataSave();
		await vi.advanceTimersByTimeAsync(600);
		expect(mocks.api.setPackMetadata).not.toHaveBeenCalled();
	});
});
