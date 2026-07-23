import { store } from './store.svelte.js';
import { taskFeedback } from './taskFeedback.svelte.js';

export function openMediaPreview(id: number): boolean {
	if (store.saveBlocksPreviews) {
		taskFeedback.warning('preview', 'Preview unavailable while the pack is being saved');
		return false;
	}
	taskFeedback.dismiss('preview');
	store.openedId = id;
	return true;
}
