<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	import Field from '$ui/Field.svelte';
	import Select from '$ui/Select.svelte';
	import { store } from './store.svelte.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { fields, mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import type { ContentGroup } from './types.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';

	const stored = query(keys.contentGroups, api.getContentGroups);
	const groups = $derived(stored.current ?? []);
	const availableToggleTags = $derived(
		store.allTags.filter((tag) => !groups.some((group) => group.tags.includes(tag)))
	);
	const invalidates = [keys.contentGroups, keys.summary];

	let quickCreateTag = $state('');
	let removing = $state<ContentGroup | null>(null);
	let list = $state<ContentList<ContentGroup>>();

	function capitalize(value: string): string {
		return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1);
	}

	/**
	 * Applies `change` to this group and sends it.
	 *
	 * Accumulated into one draft per group, for the same reason as the pools: the command sends the
	 * group whole, so a description edit built from the last fetched copy would revert the name
	 * typed a moment earlier.
	 */
	function write(
		group: ContentGroup,
		change: (draft: ContentGroup) => void,
		label: string,
		debounce = false
	) {
		fields.edit<ContentGroup>({
			entity: `content-group:${group.id}`,
			base: () => ({ ...group, tags: [...group.tags] }),
			change,
			label,
			invalidates,
			send: (draft) => api.updateContentGroup(group.id, draft, label),
			debounce
		});
	}

	/** What a group looks like right now: the author's unsent edit if there is one, else stored. */
	function shown(group: ContentGroup): ContentGroup {
		return fields.draftFor<ContentGroup>(`content-group:${group.id}`) ?? group;
	}

	// Awaited: `ContentList` reveals and focuses the new entry once this resolves. See
	// `TextPoolEditor.addItem`.
	async function addGroup() {
		await mutate(
			() =>
				api.addContentGroup(
					{
						id: `group-${Date.now()}`,
						label: 'New group',
						description: null,
						tags: [],
						enabled_by_default: true
					},
					'Add content group'
				),
			{ label: 'Add content group', invalidates }
		);
	}

	/** The shortcut in the bar: an existing tag becomes a group people can switch off. */
	async function quickCreateFromTag(tag: string) {
		if (!tag) return;
		quickCreateTag = '';
		await mutate(
			() =>
				api.addContentGroup(
					{
						id: tag,
						label: capitalize(tag),
						description: null,
						tags: [tag],
						enabled_by_default: true
					},
					'Add content group'
				),
			{ label: 'Add content group', invalidates }
		);
		await list?.reveal();
	}

	function removeGroup(index: number) {
		const group = groups[index];
		if (!group) return;
		if (group.tags.length > 0 || group.description || group.label !== 'New group') {
			removing = group;
			return;
		}
		void mutate(() => api.removeContentGroup(group.id, 'Remove content group'), {
			label: 'Remove content group',
			invalidates
		});
	}

	function confirmRemove() {
		if (!removing) return;
		const group = removing;
		removing = null;
		void mutate(() => api.removeContentGroup(group.id, 'Remove content group'), {
			label: 'Remove content group',
			invalidates
		});
	}
</script>

<ContentList
	bind:this={list}
	label="Content groups"
	items={groups}
	entryLabel="Group"
	addLabel="Add group"
	focusSelector="input"
	toolbarWhenEmpty
	removeLabel={(group, index) => `Remove group ${index + 1}: ${group.label}`}
	onadd={addGroup}
	onremove={removeGroup}
>
	{#snippet toolbar()}
		{#if availableToggleTags.length > 0}
			<Select
				class="w-48"
				size="compact"
				hideLabel
				label="Make an existing tag toggleable"
				value={quickCreateTag}
				options={[
					{ value: '', label: 'Make tag toggleable…' },
					...availableToggleTags.map((tag) => ({ value: tag, label: tag }))
				]}
				onchange={(value) => {
					quickCreateTag = value;
					void quickCreateFromTag(value);
				}}
			/>
		{/if}
	{/snippet}
	{#snippet empty()}
		<EmptyState
			title="No content groups yet"
			description="Create a group when you want people to opt in or out of related tagged content."
		/>
	{/snippet}
	{#snippet fields(group, index)}
		<div class="flex flex-1 flex-col gap-3">
			<Field
				label="Group name"
				size="compact"
				value={shown(group).label}
				placeholder="Label"
				oninput={(value) => write(group, (draft) => (draft.label = value), 'Edit group name', true)}
			/>
			<Field
				label="Description (optional)"
				size="compact"
				value={shown(group).description ?? ''}
				placeholder="Explain what this group contains"
				oninput={(value) =>
					write(
						group,
						// A blank description is the absence of one, not an empty string.
						(draft) => (draft.description = value.trim() ? value : null),
						'Edit group description',
						true
					)}
			/>
		</div>
		<TagPicker
			tags={shown(group).tags}
			id={`content-group-${index}`}
			onchange={(tags, label) => write(group, (draft) => (draft.tags = tags), label)}
		/>
		<label class="flex items-center gap-2">
			<Checkbox
				checked={shown(group).enabled_by_default}
				ariaLabel="Enabled by default"
				onchange={(checked) =>
					write(group, (draft) => (draft.enabled_by_default = checked), 'Change group default')}
			/>
			<span class="text-text text-xs">Enabled by default</span>
		</label>
	{/snippet}
</ContentList>

{#if removing}
	<Dialog
		title={`Remove “${removing.label}”?`}
		description="This content group will be removed. Its tags and matching media will remain available, and you can undo this change from the editor history."
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			{ label: 'Remove group', destructive: true, onclick: confirmRemove }
		]}
		onclose={() => (removing = null)}
	/>
{/if}
