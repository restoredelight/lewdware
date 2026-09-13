<script lang="ts">
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import DebouncedField from './DebouncedField.svelte';
	import { keys, query } from './query.svelte.js';
	import type { WebLink } from './types.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Field from '$ui/Field.svelte';
	import Dialog from '$ui/Dialog.svelte';

	const stored = query(keys.webLinks, api.webLink.get);
	const links = $derived(stored.current ?? []);
	const invalidates = [keys.webLinks, keys.summary];

	let removing = $state<WebLink | null>(null);

	/** Sends one field of one link. Nothing else about the link travels with it. */
	function write(run: () => Promise<void>, label: string) {
		void mutate(run, { label, invalidates });
	}

	async function addLink() {
		await mutate(() => api.webLink.add({ url: '', tags: [] }, 'Add web link'), {
			label: 'Add web link',
			invalidates
		});
	}

	function removeLink(index: number) {
		const link = links[index];
		if (!link) return;
		if (link.url || link.tags.length > 0) {
			removing = link;
			return;
		}
		write(() => api.webLink.remove(link.id, 'Remove web link'), 'Remove web link');
	}

	function confirmRemove() {
		if (!removing) return;
		const link = removing;
		removing = null;
		write(() => api.webLink.remove(link.id, 'Remove web link'), 'Remove web link');
	}
</script>

<ContentList
	label="Web links"
	items={links}
	entryLabel="Web link"
	addLabel="Add web link"
	focusSelector="input"
	onadd={addLink}
	onremove={removeLink}
>
	{#snippet empty()}
		<EmptyState
			title="No web links yet"
			description="Add a link if this pack should be able to open a page in the user’s browser."
			actionLabel="Add web link"
			onclick={addLink}
		/>
	{/snippet}
	{#snippet fields(link, index)}
		<DebouncedField
			value={link.url}
			label="Edit web link"
			{invalidates}
			oncommit={(value: string, label: string) => api.webLink.setUrl(link.id, value, label)}
		>
			{#snippet field(draft, set, commit)}
				<Field
					label="URL"
					type="url"
					size="compact"
					value={draft}
					placeholder="https://..."
					oninput={set}
					onchange={() => commit()}
				/>
			{/snippet}
		</DebouncedField>
		<TagPicker
			tags={link.tags}
			id={`web-link-${index}`}
			onchange={(tag, added, label) =>
				write(
					() =>
						added
							? api.webLink.addTag(link.id, tag, label)
							: api.webLink.removeTag(link.id, tag, label),
					label
				)}
		/>
	{/snippet}
</ContentList>

{#if removing}
	<Dialog
		title="Remove web link?"
		description="This link and its tag assignments will be removed. You can undo this change from the editor history."
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			{ label: 'Remove link', destructive: true, onclick: confirmRemove }
		]}
		onclose={() => (removing = null)}
	/>
{/if}
