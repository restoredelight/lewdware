<script lang="ts">
	import { store } from './store.svelte.js';
	import TagInput from '$ui/TagInput.svelte';

	type Props = {
		tags: string[];
		id: string;
		/**
		 * Sends the new tag list, with the undo label for what the author did.
		 *
		 * This used to be a dot-separated `path` into the behaviour document, which made one shared
		 * leaf component the write path for every "entity has tags" surface in the editor — a
		 * caption, a web link, a content group, a stage. Each of those is now its own typed command,
		 * so the parent (which knows what it owns) does the writing and this only knows it has some
		 * tags.
		 */
		onchange: (tags: string[], label: string) => void;
	};

	let { tags, id, onchange }: Props = $props();

	function addTag(t: string) {
		if (!t || tags.includes(t)) return;
		onchange([...tags, t], 'Add tag');
	}

	function removeTag(tag: string) {
		if (!tags.includes(tag)) return;
		onchange(
			tags.filter((item) => item !== tag),
			'Remove tag'
		);
	}
</script>

<TagInput {tags} suggestions={store.allTags} label="Tags" onadd={addTag} onremove={removeTag} />
