<script lang="ts">
	import type { TextItem } from './types.js';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import { store } from './store.svelte.js';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { Icon, Plus } from 'svelte-hero-icons';

	type Props = {
		title: string;
		description?: string;
		// A key into behaviour content rather than the array itself: this editor mutates the
		// pool, and mutating an unbound prop trips Svelte's ownership warning.
		poolKey: 'captions' | 'prompts' | 'notifications' | 'subliminals';
		idPrefix: string;
	};

	let { title, description, poolKey, idPrefix }: Props = $props();
	const pool = $derived(store.behaviour!.content[poolKey]);
	// The pool's address in the behaviour document. Adding and removing entries move every later
	// index, so those edits replace the array whole rather than addressing one entry.
	const poolPath = $derived(`content.${poolKey}`);
	// "Caption", "Prompt" -- what the undo list should call one of these.
	const noun = $derived(title.replace(/s$/, ''));
	let removing = $state<TextItem | null>(null);

	function addItem() {
		pool.push({ text: '', tags: [] });
		commitBehaviourEdit(poolPath, `Add ${noun.toLowerCase()}`);
	}

	function removeItem(index: number) {
		if (pool[index].text.trim() || pool[index].tags.length > 0) {
			removing = pool[index];
			return;
		}
		pool.splice(index, 1);
		commitBehaviourEdit(poolPath, `Remove ${noun.toLowerCase()}`);
	}

	function confirmRemove() {
		if (!removing) return;
		const index = pool.indexOf(removing);
		if (index >= 0) pool.splice(index, 1);
		removing = null;
		commitBehaviourEdit(poolPath, `Remove ${noun.toLowerCase()}`);
	}
</script>

<section class="flex flex-col gap-3" aria-label={title}>
	{#if description}
		<p class="text-muted text-xs">{description}</p>
	{/if}

	<div class="flex flex-col gap-2">
		{#if pool.length === 0}
			<EmptyState
				title={`No ${title.toLowerCase()} yet`}
				description={`Add the first ${title.toLowerCase().replace(/s$/, '')} to make it available to this pack.`}
				actionLabel={`Add ${title.toLowerCase().replace(/s$/, '')}`}
				onclick={addItem}
			/>
		{/if}
		{#each pool as item, index}
			<Card class="flex flex-col gap-3 p-3">
				<div class="flex items-center justify-between">
					<span class="text-muted font-mono text-[11px] font-semibold"
						>{title.replace(/s$/, '')} {index + 1}</span
					><Button
						size="compact"
						variant="destructive"
						class="!h-7"
						onclick={() => removeItem(index)}>Remove</Button
					>
				</div>
				<div class="flex items-start gap-2">
					<label class="sr-only" for={`${idPrefix}-text-${index}`}
						>{title.replace(/s$/, '')} text</label
					>
					<textarea
						id={`${idPrefix}-text-${index}`}
						bind:value={item.text}
						oninput={() =>
							editBehaviourField(`${poolPath}.${index}.text`, `Edit ${noun.toLowerCase()}`)}
						rows={2}
						placeholder="Text"
						class="border-border bg-bg text-text flex-1 resize-none rounded border px-2 py-1 text-xs
"></textarea>
				</div>
				<TagPicker
					tags={item.tags}
					id={`${idPrefix}-${index}`}
					path={`${poolPath}.${index}.tags`}
					onchange={(tags) => (item.tags = tags)}
				/>
			</Card>
		{/each}
	</div>

	{#if pool.length > 0}<Button size="compact" class="self-start" onclick={addItem}
			><span class="h-4 w-4"><Icon src={Plus} mini /></span> Add {title
				.toLowerCase()
				.replace(/s$/, '')}</Button
		>{/if}
</section>

{#if removing}
	<Dialog
		title={`Remove ${title.toLowerCase().replace(/s$/, '')}?`}
		description="This entry and its tag assignments will be removed. You can undo this change from the editor history."
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			{ label: 'Remove', destructive: true, onclick: confirmRemove }
		]}
		onclose={() => (removing = null)}
	/>
{/if}
