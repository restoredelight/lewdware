<script lang="ts">
	// The shape `TimelineEditor` renders: a stage taken out of a fetched list, its id handed down,
	// and a side effect that must follow the *stage* rather than the fetch.
	import { onChange } from './onChange.svelte.js';

	type Props = {
		id: string;
		stages?: { id: string }[];
		onrun: (value: string) => void;
		/** The version this replaced, kept so a test can show it really does over-fire. */
		onbare?: (value: string) => void;
	};
	let { id, stages = $bindable([{ id }]), onrun, onbare }: Props = $props();

	const stage = $derived(stages.find((item) => item.id === id) ?? { id });
	onChange(
		() => stage.id,
		(value) => onrun(value)
	);

	$effect(() => {
		const value = stage.id;
		onbare?.(value);
	});
</script>
