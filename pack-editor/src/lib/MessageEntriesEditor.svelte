<script lang="ts">
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

	type MessageKind = 'caption' | 'prompt' | 'notification';
	type MessageRecord = {
		id: number;
		text: string;
		tags: string[];
		timeout_seconds?: number;
		summary?: string;
	};

	type Props = {
		title: string;
		kind: MessageKind;
		idPrefix: string;
	};

	let { title, kind, idPrefix }: Props = $props();

	// Each entry carries the row id the backend addresses it by, so an edit names the entry the
	// author is looking at rather than the position it happened to be in when the view rendered.
	const stored = query(
		() => entryKey(kind),
		() => getEntries(kind)
	);
	const entries = $derived(stored.current ?? []);
	// Notifications are the one message whose entry is two fields: the desktop notification's title
	// and its body. Everywhere else `text` is the whole entry, and an unlabelled box is clearer
	// than a labelled one.
	const titled = $derived(kind === 'notification');
	// "Caption", "Prompt" -- what the undo list should call one of these.
	const noun = $derived(title.replace(/s$/, ''));
	let removing = $state<MessageRecord | null>(null);

	const invalidates = $derived([entryKey(kind), keys.summary]);

	function entryKey(entryKind: MessageKind): string {
		switch (entryKind) {
			case 'caption':
				return keys.captions;
			case 'prompt':
				return keys.prompts;
			case 'notification':
				return keys.notifications;
		}
	}

	async function getEntries(entryKind: MessageKind): Promise<MessageRecord[]> {
		switch (entryKind) {
			case 'caption':
				return api.caption.get();
			case 'prompt':
				return api.prompt.get();
			case 'notification':
				return api.notification.get();
		}
	}

	/** Sends one field of one entry. Nothing else about the entry travels with it. */
	function write(run: () => Promise<void>, label: string) {
		void mutate(run, { label, invalidates });
	}

	function setText(id: number, text: string, label: string) {
		switch (kind) {
			case 'caption':
				return api.caption.setText(id, text, label);
			case 'prompt':
				return api.prompt.setText(id, text, label);
			case 'notification':
				return api.notification.setBody(id, text, label);
		}
	}

	// Awaited, not fired and forgotten: `ContentList` reveals and focuses the new entry once this
	// resolves, and an entry that does not exist yet cannot be revealed — it would scroll to the
	// previous last card instead.
	async function addItem() {
		const label = `Add ${noun.toLowerCase()}`;
		const add = () => {
			switch (kind) {
				case 'caption':
					return api.caption.add({ text: '', tags: [] }, label);
				case 'prompt':
					return api.prompt.add({ text: '', tags: [] }, label);
				case 'notification':
					return api.notification.add({ text: '', tags: [] }, label);
			}
		};
		await mutate(add, {
			label,
			invalidates
		});
	}

	function removeEntry(id: number, label: string): Promise<void> {
		switch (kind) {
			case 'caption':
				return api.caption.remove(id, label);
			case 'prompt':
				return api.prompt.remove(id, label);
			case 'notification':
				return api.notification.remove(id, label);
		}
	}

	function setTag(id: number, tag: string, added: boolean, label: string): Promise<void> {
		switch (kind) {
			case 'caption':
				return added
					? api.caption.addTag(id, tag, label)
					: api.caption.removeTag(id, tag, label);
			case 'prompt':
				return added
					? api.prompt.addTag(id, tag, label)
					: api.prompt.removeTag(id, tag, label);
			case 'notification':
				return added
					? api.notification.addTag(id, tag, label)
					: api.notification.removeTag(id, tag, label);
		}
	}

	function removeItem(index: number) {
		const item = entries[index];
		if (!item) return;
		if (item.text.trim() || item.summary?.trim() || item.tags.length > 0) {
			removing = item;
			return;
		}
		write(
			() => removeEntry(item.id, `Remove ${noun.toLowerCase()}`),
			`Remove ${noun.toLowerCase()}`
		);
	}

	function confirmRemove() {
		if (!removing) return;
		const item = removing;
		removing = null;
		write(
			() => removeEntry(item.id, `Remove ${noun.toLowerCase()}`),
			`Remove ${noun.toLowerCase()}`
		);
	}
</script>

<ContentList
	label={title}
	items={entries}
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
					api.notification.setSummary(
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
				oncommit={(value: string, label: string) => setText(item.id, value, label)}
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
			onchange={(tag, added, label) =>
				write(() => setTag(item.id, tag, added, label), label)}
		/>
		{#if kind === 'prompt'}
			<DebouncedField
				value={item.timeout_seconds ?? null}
				label="Edit prompt time limit"
				{invalidates}
				oncommit={(seconds: number | null, label: string) =>
					api.prompt.setTimeout(item.id, seconds, label)}
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
