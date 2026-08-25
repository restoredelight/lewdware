<script lang="ts">
	import { store } from './store.svelte.js';
	import TagInput from '$ui/TagInput.svelte';

	type Props = {
		tags: string[];
		id: string;
		/**
		 * Sends the one chip that changed, with the undo label for what the author did.
		 *
		 * The *chip* rather than the resulting list, which is what this used to send. A list is
		 * built from what was last fetched, so removing two chips in quick succession sends
		 * "everything except A" and then "everything except B" — and the second puts A back. Naming
		 * the item makes the two edits commute.
		 *
		 * Before that it was a dot-separated `path` into the behaviour document, which made one
		 * shared leaf component the write path for every "entity has tags" surface in the editor.
		 * The parent owns the writing now; this only knows it has some tags.
		 */
		onchange: (tag: string, added: boolean, label: string) => void;
	};

	let { tags, id, onchange }: Props = $props();

	function addTag(tag: string) {
		if (!tag || tags.includes(tag)) return;
		onchange(tag, true, 'Add tag');
	}

	function removeTag(tag: string) {
		if (!tags.includes(tag)) return;
		onchange(tag, false, 'Remove tag');
	}
</script>

<TagInput {tags} suggestions={store.allTags} label="Tags" onadd={addTag} onremove={removeTag} />
