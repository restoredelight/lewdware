import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fields } from './mutate.svelte.js';
import { resetQueries } from './query.svelte.js';
import { store } from './store.svelte.js';
import type { BehaviourOutcome } from './types.js';

vi.mock('./history.svelte.js', () => ({ history: { record: () => {} } }));
vi.mock('$ui/taskFeedback.svelte.js', () => ({
	taskFeedback: { error: () => {}, dismiss: () => {} }
}));

const ok: BehaviourOutcome = { deleted_ids: [], removed_tags: [], renamed_tags: [] };

/** What the pool holds. Every edit sends the entry whole, so a stale part reverts it. */
interface Entry {
	text: string;
	summary?: string;
	tags: string[];
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
	resetQueries();
	fields.cancel();
	store.openPack('pack-id', 'Test', [], [], []);
});

describe('the edit buffer', () => {
	// The bug: each command used to be built from the last *fetched* copy the moment the field
	// changed. Editing a title and then its message sent the message with the title as it was
	// before, silently reverting it.
	it('accumulates several fields of one entry into a single draft', async () => {
		const stored: Entry = { text: 'body', summary: 'title', tags: [] };
		const send = vi.fn().mockResolvedValue(ok);

		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ ...stored, tags: [...stored.tags] }),
			change: (draft) => (draft.summary = 'new title'),
			label: 'Edit notification title',
			invalidates: [],
			send,
			debounce: true
		});
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ ...stored, tags: [...stored.tags] }),
			change: (draft) => (draft.text = 'new body'),
			label: 'Edit notification',
			invalidates: [],
			send,
			debounce: true
		});
		await fields.flush();

		expect(send).toHaveBeenCalledTimes(1);
		expect(send.mock.calls[0][0]).toMatchObject({ summary: 'new title', text: 'new body' });
	});

	// An immediate control — a tag chip, a toggle — collides the same way with text still being
	// typed. It merges into the same draft rather than racing it.
	it('merges an immediate change into text that has not been sent yet', async () => {
		const stored: Entry = { text: 'body', tags: [] };
		const send = vi.fn().mockResolvedValue(ok);
		const edit = (change: (draft: Entry) => void, debounce: boolean) =>
			fields.edit<Entry>({
				entity: 'caption:1',
				base: () => ({ ...stored, tags: [...stored.tags] }),
				change,
				label: 'Edit caption',
				invalidates: [],
				send,
				debounce
			});

		edit((draft) => (draft.text = 'typed'), true);
		edit((draft) => draft.tags.push('imp'), false);
		await settle();

		expect(send).toHaveBeenCalledTimes(1);
		expect(send.mock.calls[0][0]).toMatchObject({ text: 'typed', tags: ['imp'] });
	});

	// Two entries are two author actions, and each deserves its own undo entry.
	it('sends the previous entry before starting a different one', async () => {
		const send = vi.fn().mockResolvedValue(ok);
		const edit = (entity: string, text: string) =>
			fields.edit<Entry>({
				entity,
				base: () => ({ text: '', tags: [] }),
				change: (draft) => (draft.text = text),
				label: 'Edit caption',
				invalidates: [],
				send,
				debounce: true
			});

		edit('caption:1', 'first');
		edit('caption:2', 'second');
		await fields.flush();

		expect(send).toHaveBeenCalledTimes(2);
		expect(send.mock.calls[0][0]).toMatchObject({ text: 'first' });
		expect(send.mock.calls[1][0]).toMatchObject({ text: 'second' });
	});

	it('shows the unsent draft in preference to what was fetched', () => {
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ text: 'stored', tags: [] }),
			change: (draft) => (draft.text = 'typed'),
			label: 'Edit caption',
			invalidates: [],
			send: vi.fn().mockResolvedValue(ok),
			debounce: true
		});

		expect(fields.draftFor<Entry>('caption:1')?.text).toBe('typed');
		expect(fields.draftFor<Entry>('caption:2')).toBeUndefined();
	});

	// Saving and closing both wait on `flush`. If it resolved despite a failed write, the pack
	// would be written without the value the author can see, and the save would then report the
	// pack as having no unsaved changes.
	it('rejects when a write failed, so saving and closing cannot carry on', async () => {
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ text: '', tags: [] }),
			change: (draft) => (draft.text = 'unsaveable'),
			label: 'Edit caption',
			invalidates: [],
			send: vi.fn().mockRejectedValue(new Error('disk on fire')),
			debounce: true
		});

		await expect(fields.flush()).rejects.toThrow();
	});

	it('resolves when the write succeeded', async () => {
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ text: '', tags: [] }),
			change: (draft) => (draft.text = 'fine'),
			label: 'Edit caption',
			invalidates: [],
			send: vi.fn().mockResolvedValue(ok),
			debounce: true
		});

		await expect(fields.flush()).resolves.toBeUndefined();
	});

	// A revert replaces what the buffer is holding an edit against; sending it afterwards would
	// write it back over the state the author asked to return to.
	it('forgets a pending edit when cancelled', async () => {
		const send = vi.fn().mockResolvedValue(ok);
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ text: '', tags: [] }),
			change: (draft) => (draft.text = 'abandoned'),
			label: 'Edit caption',
			invalidates: [],
			send,
			debounce: true
		});
		fields.cancel();
		await fields.flush();

		expect(send).not.toHaveBeenCalled();
		expect(fields.draftFor('caption:1')).toBeUndefined();
	});
});

describe('what a field shows while its edit is on its way', () => {
	const pendingSend = () => {
		let release!: () => void;
		const send = vi.fn(
			() => new Promise<BehaviourOutcome>((resolve) => (release = () => resolve(ok)))
		);
		return { send, release: () => release() };
	};

	const type = (text: string, send: () => Promise<BehaviourOutcome>) =>
		fields.edit<Entry>({
			entity: 'caption:1',
			base: () => ({ text: 'stored', tags: [] }),
			change: (draft) => (draft.text = text),
			label: 'Edit caption',
			invalidates: [],
			send,
			debounce: true
		});

	// The reported symptom: the field snapped back to its previous value for the length of the
	// round trip, because the draft was dropped when the write was merely *issued*.
	it('keeps showing the typed value until the write has landed', async () => {
		const { send, release } = pendingSend();
		type('typed', send);
		const flushed = fields.flush();
		await settle();

		expect(fields.draftFor<Entry>('caption:1')?.text).toBe('typed');

		release();
		await flushed;
		// Once it has landed, the refetch behind it is current, so the field reads from that.
		expect(fields.draftFor('caption:1')).toBeUndefined();
	});

	// The other half: a keystroke during the round trip must not rebuild the draft from the value
	// the backend had before the write, which is what lost characters.
	it('keeps the newer text when the author types while a write is in flight', async () => {
		const { send, release } = pendingSend();
		type('first', send);
		const flushed = fields.flush();
		await settle();

		type('first and more', send);
		release();
		await flushed;

		expect(fields.draftFor<Entry>('caption:1')?.text).toBe('first and more');
	});
});
