<script lang="ts">
	import type { PoolKind, TextItem, TextItemRow } from './types.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { fields, mutate } from './mutate.svelte.js';
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
		() => api.getTextPool(poolKey)
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

	/**
	 * Applies `change` to this entry and sends it.
	 *
	 * Every change to one entry accumulates into a single draft — the text, the title and the tag
	 * chips alike — because the command sends the entry whole. Building each one from the last
	 * *fetched* copy would mean editing a title and then its message sent the message with the
	 * title as it was before, reverting it.
	 */
	function write(
		item: TextItemRow,
		change: (draft: TextItem) => void,
		label: string,
		debounce = false
	) {
		fields.edit<TextItem>({
			entity: `${poolKey}:${item.id}`,
			base: () => ({
				text: item.text,
				tags: [...item.tags],
				timeout_seconds: item.timeout_seconds,
				summary: item.summary
			}),
			change,
			label,
			invalidates,
			send: (draft) => api.updateTextItem(item.id, draft, label),
			debounce
		});
	}

	/** What an entry looks like right now: the author's unsent edit if there is one, else stored. */
	function shown(item: TextItemRow): TextItem {
		return fields.draftFor<TextItem>(`${poolKey}:${item.id}`) ?? item;
	}

	// Awaited, not fired and forgotten: `ContentList` reveals and focuses the new entry once this
	// resolves, and an entry that does not exist yet cannot be revealed — it would scroll to the
	// previous last card instead.
	async function addItem() {
		await mutate(
			() => api.addTextItem(poolKey, { text: '', tags: [] }, `Add ${noun.toLowerCase()}`),
			{ label: `Add ${noun.toLowerCase()}`, invalidates }
		);
	}

	function removeItem(index: number) {
		const item = pool[index];
		if (!item) return;
		if (item.text.trim() || item.summary?.trim() || item.tags.length > 0) {
			removing = item;
			return;
		}
		void mutate(() => api.removeTextItem(item.id, `Remove ${noun.toLowerCase()}`), {
			label: `Remove ${noun.toLowerCase()}`,
			invalidates
		});
	}

	function confirmRemove() {
		if (!removing) return;
		const item = removing;
		removing = null;
		void mutate(() => api.removeTextItem(item.id, `Remove ${noun.toLowerCase()}`), {
			label: `Remove ${noun.toLowerCase()}`,
			invalidates
		});
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
				value={shown(item).summary ?? ''}
				oninput={(value) => {
					// Stored only when it says something: a blank title is the absence of one, and
					// the notification is shown body-only.
					write(
						item,
						(draft) => (draft.summary = value.trim() ? value : undefined),
						'Edit notification title',
						true
					);
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
				value={shown(item).text}
				oninput={(event) =>
					write(
						item,
						(draft) => (draft.text = event.currentTarget.value),
						`Edit ${noun.toLowerCase()}`,
						true
					)}
				rows={2}
				placeholder={titled ? undefined : 'Text'}
				class="border-border bg-bg text-text w-full resize-none rounded border px-2 py-1 text-xs"
			></textarea>
		</div>
		<TagPicker
			tags={shown(item).tags}
			id={`${idPrefix}-${index}`}
			onchange={(tags, label) => write(item, (draft) => (draft.tags = tags), label)}
		/>
		{#if poolKey === 'prompt'}
			<NumberField
				label="Time limit"
				description={shown(item).timeout_seconds == null
					? `Automatic: ${automaticPromptTimeout(shown(item).text)} seconds based on this prompt's length.`
					: 'Clear this value to use the automatic limit based on prompt length.'}
				placeholder="Automatic"
				suffix="s"
				min={1}
				step={1}
				value={shown(item).timeout_seconds ?? null}
				oninput={(seconds) => {
					// Clearing the field is how an author asks for the automatic limit back.
					write(
						item,
						(draft) => (draft.timeout_seconds = seconds ?? undefined),
						'Edit prompt time limit',
						true
					);
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
