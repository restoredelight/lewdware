<script lang="ts">
	import { store } from './store.svelte.js';
	import { scheduleBehaviourSave } from './behaviourSave.svelte.js';
	import TagInput from '$ui/TagInput.svelte';

	type Props = {
		tags: string[];
		id: string;
		// The parent owns the array: mutating a plain prop trips Svelte's ownership warning.
		onchange: (tags: string[]) => void;
	};

	let { tags, id, onchange }: Props = $props();

	function addTag(t: string) {
		if (!t || tags.includes(t)) return;
		onchange([...tags, t]);
		scheduleBehaviourSave();
	}

	function removeTag(tag: string) {
		if (!tags.includes(tag)) return;
		onchange(tags.filter((item) => item !== tag));
		scheduleBehaviourSave();
	}
</script>

<TagInput {tags} suggestions={store.allTags} label="Tags" onadd={addTag} onremove={removeTag} />
