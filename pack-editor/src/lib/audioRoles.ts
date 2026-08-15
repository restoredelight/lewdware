import { api } from './api.js';
import { history } from './history.svelte.js';
import { store } from './store.svelte.js';
import { POPUP_AUDIO_TAG } from './tags.js';

export type AudioRole = 'background' | 'popup';

export function audioRole(tags: string[]): AudioRole {
	return tags.includes(POPUP_AUDIO_TAG) ? 'popup' : 'background';
}

export async function setAudioRole(ids: number[], role: AudioRole): Promise<void> {
	if (ids.length === 0) return;
	const popup = role === 'popup';
	const affected = store.files.filter(
		(file) =>
			ids.includes(file.id) && file.file_info.type === 'audio' && audioRole(file.tags) !== role
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
	history.record({
		label: `Move ${affected.length === 1 ? `“${affected[0].file_name}”` : `${affected.length} audio files`} to ${role === 'popup' ? 'Popup' : 'Background'}`
	});
}
