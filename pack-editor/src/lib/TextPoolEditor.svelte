<script lang="ts">
	import type { TextItem } from './types.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import { store } from './store.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import { automaticPromptTimeout } from './promptTimeout.js';

	type Props = {
		title: string;
		// A key into behaviour content rather than the array itself: this editor mutates the
		// pool, and mutating an unbound prop trips Svelte's ownership warning.
		poolKey: 'captions' | 'prompts' | 'notifications' | 'subliminals';
		idPrefix: string;
	};

	let { title, poolKey, idPrefix }: Props = $props();
	const pool = $derived(store.behaviour!.content[poolKey]);
	// Notifications are the one pool whose entry is two fields: the desktop notification's title
	// and its body. Everywhere else `text` is the whole entry, and an unlabelled box is clearer
	// than a labelled one.
	const titled = $derived(poolKey === 'notifications');
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
		if (pool[index].text.trim() || pool[index].summary?.trim() || pool[index].tags.length > 0) {
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

<ContentList
	label={title}
	items={pool}
	entryLabel={noun}
	addLabel={`Add ${noun.toLowerCase()}`}
	focusSelector={titled ? 'input' : 'textarea'}
	onadd={addItem}
	onremove={removeItem}
>
	{#snippet empty()}
		<EmptyState
			title={`No ${title.toLowerCase()} yet`}
			description={`Add the first ${noun.toLowerCase()} to make it available to this pack.`}
			actionLabel={`Add ${noun.toLowerCase()}`}
			onclick={addItem}
		/>
	{/snippet}
	{#snippet fields(item, index)}
		{#if titled}
			<Field
				label="Title"
				size="compact"
				placeholder="Optional"
				value={item.summary ?? ''}
				oninput={(value) => {
					// Stored only when it says something: a blank title is the absence of one, and
					// the notification is shown body-only.
					if (value.trim()) item.summary = value;
					else delete item.summary;
					editBehaviourField(`${poolPath}.${index}.summary`, 'Edit notification title');
				}}
			/>
		{/if}
		<div class="flex flex-col gap-[5px]">
			<label
				class={titled ? 'text-text text-xs font-semibold' : 'sr-only'}
				for={`${idPrefix}-text-${index}`}>{titled ? 'Message' : `${noun} text`}</label
			>
			<textarea
				id={`${idPrefix}-text-${index}`}
				bind:value={item.text}
				oninput={() =>
					editBehaviourField(`${poolPath}.${index}.text`, `Edit ${noun.toLowerCase()}`)}
				rows={2}
				placeholder={titled ? undefined : 'Text'}
				class="border-border bg-bg text-text w-full resize-none rounded border px-2 py-1 text-xs"
			></textarea>
		</div>
		<TagPicker
			tags={item.tags}
			id={`${idPrefix}-${index}`}
			path={`${poolPath}.${index}.tags`}
			onchange={(tags) => (item.tags = tags)}
		/>
		{#if poolKey === 'prompts'}
			<NumberField
				label="Time limit"
				description={item.timeout_seconds == null
					? `Automatic: ${automaticPromptTimeout(item.text)} seconds based on this prompt's length.`
					: 'Clear this value to use the automatic limit based on prompt length.'}
				placeholder="Automatic"
				suffix="s"
				min={1}
				step={1}
				value={item.timeout_seconds ?? null}
				oninput={(seconds) => {
					// Clearing the field is how an author asks for the automatic limit back.
					if (seconds === null) delete item.timeout_seconds;
					else item.timeout_seconds = seconds;
					editBehaviourField(`${poolPath}.${index}.timeout_seconds`, 'Edit prompt time limit');
				}}
			/>
		{/if}
	{/snippet}
</ContentList>

{#if removing}
	<Dialog
		title={`Remove ${noun.toLowerCase()}?`}
		description="This entry and its tag assignments will be removed. You can undo this change from the editor history."
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			{ label: 'Remove', destructive: true, onclick: confirmRemove }
		]}
		onclose={() => (removing = null)}
	/>
{/if}
