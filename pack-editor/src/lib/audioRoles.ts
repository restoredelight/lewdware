import { api } from './api.js';
import { history } from './history.svelte.js';
import { invalidate, keys } from './query.svelte.js';
import { store } from './store.svelte.js';
import { POPUP_AUDIO_TAG } from './tags.js';
import type { MediaFile } from './types.js';

export type AudioRole = 'background' | 'popup';

export function audioRole(file: Pick<MediaFile, 'audio_role'>): AudioRole {
	return file.audio_role ?? 'background';
}

export async function setAudioRole(ids: number[], role: AudioRole): Promise<void> {
	if (ids.length === 0) return;
	const popup = role === 'popup';
	const affected = store.files.filter(
		(file) => ids.includes(file.id) && file.file_info.type === 'audio' && audioRole(file) !== role
	);
	if (affected.length === 0) return;

	await api.setPopupAudio(
		affected.map((file) => file.id),
		popup
	);
	if (popup)
		store.addTagToFiles(
			affected.map((file) => file.id),
			POPUP_AUDIO_TAG,
			true
		);
	else
		store.removeTagFromFiles(
			affected.map((file) => file.id),
			POPUP_AUDIO_TAG,
			true
		);
	await invalidate(keys.stageMemberships);
	history.record({
		label: `Move ${affected.length === 1 ? `"${affected[0].file_name}"` : `${affected.length} audio files`} to ${role === 'popup' ? 'Popup' : 'Background'}`
	});
}
