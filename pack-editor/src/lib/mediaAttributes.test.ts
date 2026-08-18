import { describe, expect, it, beforeEach, vi } from 'vitest';
import { store } from './store.svelte.js';
import {
	audioAttributes,
	editAudioAttributes,
	editManyPopupAttributes,
	editPopupAttributes,
	popupAttributes,
	sharedValue
} from './mediaAttributes.js';
import type { Behaviour } from './types.js';

const edits: { path: string; label: string }[] = [];

vi.mock('./behaviourSave.svelte.js', () => ({
	commitBehaviourEdit: (path: string, label: string) => edits.push({ path, label })
}));

function behaviour(): Behaviour {
	return {
		version: 3,
		content: {
			popups: {},
			audio: {},
			content_groups: [],
			captions: [],
			prompts: [],
			notifications: [],
			subliminals: [],
			web_links: []
		},
		experience: null
	} as unknown as Behaviour;
}

describe('per-file behaviour attributes', () => {
	beforeEach(() => {
		edits.length = 0;
		store.behaviour = behaviour();
	});

	it('patches the whole entry, since a field of one that does not exist is unreachable', () => {
		editPopupAttributes(42, { scale: 2 }, 'Set popup size');
		expect(store.behaviour!.content.popups['42']).toEqual({ scale: 2 });
		expect(edits).toEqual([{ path: 'content.popups.42', label: 'Set popup size' }]);
	});

	it('merges into an existing entry rather than replacing it', () => {
		editPopupAttributes(42, { scale: 2 }, 'Set popup size');
		editPopupAttributes(42, { weight: 3 }, 'Set popup frequency');
		expect(store.behaviour!.content.popups['42']).toEqual({ scale: 2, weight: 3 });
	});

	it('clears a field to absent rather than to a zero', () => {
		editPopupAttributes(42, { scale: 2, weight: 3 }, 'Set');
		editPopupAttributes(42, { scale: undefined }, 'Clear');
		expect(store.behaviour!.content.popups['42']).toEqual({ weight: 3 });
		expect('scale' in store.behaviour!.content.popups['42']).toBe(false);
	});

	it('removes an entry once nothing is left to say, replacing the section', () => {
		editPopupAttributes(42, { scale: 2 }, 'Set');
		edits.length = 0;
		editPopupAttributes(42, { scale: undefined }, 'Clear');

		expect(store.behaviour!.content.popups).toEqual({});
		// Removing a key cannot be said by patching the entry's own path -- null there would fail
		// to parse as an entry -- so the section goes whole.
		expect(edits).toEqual([{ path: 'content.popups', label: 'Clear' }]);
	});

	it('treats an empty pairing list as nothing said', () => {
		editPopupAttributes(42, { audio: [7] }, 'Pair');
		expect(store.behaviour!.content.popups['42']).toEqual({ audio: [7] });
		editPopupAttributes(42, { audio: [] }, 'Unpair');
		expect(store.behaviour!.content.popups).toEqual({});
	});

	it('keeps the two sections apart', () => {
		editAudioAttributes(7, { volume: 0.5 }, 'Set volume');
		expect(audioAttributes(7)).toEqual({ volume: 0.5 });
		expect(store.behaviour!.content.audio['7']).toEqual({ volume: 0.5 });
		expect(popupAttributes(7)).toEqual({});
		expect(edits[0].path).toBe('content.audio.7');
	});

	it('applies a change across a selection under one label', () => {
		editManyPopupAttributes([1, 2, 3], { scale: 1.5 }, 'Set popup size for 3 items');
		expect(Object.keys(store.behaviour!.content.popups)).toEqual(['1', '2', '3']);
		expect(new Set(edits.map((edit) => edit.label)).size).toBe(1);
	});

	describe('sharedValue', () => {
		it('reports a shared value, and mixed where they disagree', () => {
			editPopupAttributes(1, { scale: 2 }, 'Set');
			editPopupAttributes(2, { scale: 2 }, 'Set');
			expect(sharedValue([1, 2], 'scale')).toEqual({ value: 2, mixed: false });

			editPopupAttributes(2, { scale: 3 }, 'Set');
			expect(sharedValue([1, 2], 'scale')).toEqual({ value: undefined, mixed: true });
		});

		/// "Nobody set this" and "they disagree" render differently -- Auto versus Mixed -- so the
		/// two must not collapse into the same `undefined`.
		it('distinguishes unset-everywhere from mixed', () => {
			expect(sharedValue([1, 2], 'scale')).toEqual({ value: undefined, mixed: false });

			editPopupAttributes(1, { scale: 2 }, 'Set');
			expect(sharedValue([1, 2], 'scale')).toEqual({ value: undefined, mixed: true });
		});

		it('compares pairing lists by contents, not by identity', () => {
			editPopupAttributes(1, { audio: [7, 8] }, 'Pair');
			editPopupAttributes(2, { audio: [7, 8] }, 'Pair');
			expect(sharedValue([1, 2], 'audio').mixed).toBe(false);

			editPopupAttributes(2, { audio: [7] }, 'Pair');
			expect(sharedValue([1, 2], 'audio').mixed).toBe(true);
		});
	});
});
