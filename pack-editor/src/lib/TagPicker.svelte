<script lang="ts">
	import { store } from './store.svelte.js';
	import { commitBehaviourEdit } from './behaviourSave.svelte.js';
	import TagInput from '$ui/TagInput.svelte';

	type Props = {
		tags: string[];
		id: string;
		// The parent owns the array: mutating a plain prop trips Svelte's ownership warning.
		onchange: (tags: string[]) => void;
		// Where this list lives in the behaviour document, e.g. `content.captions.2.tags`. The
		// parent knows it; this component only knows it has some tags.
		path: string;
	};

	let { tags, id, onchange, path }: Props = $props();

	function addTag(t: string) {
		if (!t || tags.includes(t)) return;
		onchange([...tags, t]);
		commitBehaviourEdit(path, 'Add tag');
	}

	function removeTag(tag: string) {
		if (!tags.includes(tag)) return;
		onchange(tags.filter((item) => item !== tag));
		commitBehaviourEdit(path, 'Remove tag');
	}
</script>

<TagInput {tags} suggestions={store.allTags} label="Tags" onadd={addTag} onremove={removeTag} />
