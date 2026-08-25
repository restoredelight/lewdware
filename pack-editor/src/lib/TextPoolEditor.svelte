<script lang="ts">
	import type { PoolKind, TextItem, TextItemRow } from './types.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import DebouncedField from './DebouncedField.svelte';
	import { keys, query } from './query.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import { automaticPromptTimeout } from './promptTimeout.js';

	type Props = {
		title: string;
		poolKey: PoolKind;
		idPrefix: string;
	};

	let { title, poolKey, idPrefix }: Props = $props();

	// Each entry carries the row id the backend addresses it by, so an edit names the entry the
	// author is looking at rather than the position it happened to be in when the view rendered.
	const stored = query(
		() => keys.textPool(poolKey),
		() => api.pool.get(poolKey)
	);
	const pool = $derived(stored.current ?? []);
	// Notifications are the one pool whose entry is two fields: the desktop notification's title
	// and its body. Everywhere else `text` is the whole entry, and an unlabelled box is clearer
	// than a labelled one.
	const titled = $derived(poolKey === 'notification');
	// "Caption", "Prompt" -- what the undo list should call one of these.
	const noun = $derived(title.replace(/s$/, ''));
	let removing = $state<TextItemRow | null>(null);

	const invalidates = $derived([keys.textPool(poolKey), keys.summary]);

	/** Sends one field of one entry. Nothing else about the entry travels with it. */
	function write(run: () => Promise<void>, label: string) {
		void mutate(run, { label, invalidates });
	}

	// Awaited, not fired and forgotten: `ContentList` reveals and focuses the new entry once this
	// resolves, and an entry that does not exist yet cannot be revealed — it would scroll to the
	// previous last card instead.
	async function addItem() {
		await mutate(() => api.pool.add(poolKey, { text: '', tags: [] }, `Add ${noun.toLowerCase()}`), {
			label: `Add ${noun.toLowerCase()}`,
			invalidates
		});
	}

	function removeItem(index: number) {
		const item = pool[index];
		if (!item) return;
		if (item.text.trim() || item.summary?.trim() || item.tags.length > 0) {
			removing = item;
			return;
		}
		write(
			() => api.pool.remove(item.id, `Remove ${noun.toLowerCase()}`),
			`Remove ${noun.toLowerCase()}`
		);
	}

	function confirmRemove() {
		if (!removing) return;
		const item = removing;
		removing = null;
		write(
			() => api.pool.remove(item.id, `Remove ${noun.toLowerCase()}`),
			`Remove ${noun.toLowerCase()}`
		);
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
			<DebouncedField
				value={item.summary ?? ''}
				label="Edit notification title"
				{invalidates}
				oncommit={(value: string, label: string) =>
					api.pool.setSummary(
						item.id,
						// Stored only when it says something: a blank title is the absence of one,
						// and the notification is shown body-only.
						value.trim() ? value : null,
						label
					)}
			>
				{#snippet field(draft, set, commit)}
					<Field
						label="Title"
						size="compact"
						placeholder="Optional"
						value={draft}
						oninput={set}
						onchange={() => commit()}
					/>
				{/snippet}
			</DebouncedField>
		{/if}
		<div class="flex flex-col gap-[5px]">
			<label
				class={titled ? 'text-text text-xs font-semibold' : 'sr-only'}
				for={`${idPrefix}-text-${index}`}>{titled ? 'Message' : `${noun} text`}</label
			>
			<DebouncedField
				value={item.text}
				label={`Edit ${noun.toLowerCase()}`}
				{invalidates}
				oncommit={(value: string, label: string) => api.pool.setText(item.id, value, label)}
			>
				{#snippet field(draft, set, commit)}
					<textarea
						id={`${idPrefix}-text-${index}`}
						value={draft}
						oninput={(event) => set(event.currentTarget.value)}
						onblur={commit}
						rows={2}
						placeholder={titled ? undefined : 'Text'}
						class="border-border bg-bg text-text w-full resize-none rounded border px-2 py-1 text-xs"
					></textarea>
				{/snippet}
			</DebouncedField>
		</div>
		<TagPicker
			tags={item.tags}
			id={`${idPrefix}-${index}`}
			onchange={(tags, label) => write(() => api.pool.setTags(item.id, tags, label), label)}
		/>
		{#if poolKey === 'prompt'}
			<DebouncedField
				value={item.timeout_seconds ?? null}
				label="Edit prompt time limit"
				{invalidates}
				oncommit={(seconds: number | null, label: string) =>
					api.pool.setTimeout(item.id, seconds, label)}
			>
				{#snippet field(draft, set, commit)}
					<NumberField
						label="Time limit"
						description={draft == null
							? `Automatic: ${automaticPromptTimeout(item.text)} seconds based on this prompt's length.`
							: 'Clear this value to use the automatic limit based on prompt length.'}
						placeholder="Automatic"
						suffix="s"
						min={1}
						step={1}
						value={draft}
						oninput={set}
						onchange={() => commit()}
					/>
				{/snippet}
			</DebouncedField>
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
