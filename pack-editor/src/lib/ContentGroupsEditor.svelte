<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	import Field from '$ui/Field.svelte';
	import Select from '$ui/Select.svelte';
	import { store } from './store.svelte.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';

	const groups = $derived(store.behaviour!.content.content_groups);
	const availableToggleTags = $derived(
		store.allTags.filter((tag) => !groups.some((group) => group.tags.includes(tag)))
	);
	// Adding or removing a group moves every later index, so those edits replace the array whole
	// rather than addressing one entry.
	const GROUPS = 'content.content_groups';

	let quickCreateTag = $state('');
	let removing = $state<(typeof groups)[number] | null>(null);
	let list = $state<ContentList<(typeof groups)[number]>>();

	function capitalize(value: string): string {
		return value.length === 0 ? value : value[0].toUpperCase() + value.slice(1);
	}

	function addGroup() {
		groups.push({
			id: `group-${Date.now()}`,
			label: 'New group',
			description: null,
			tags: [],
			enabled_by_default: true
		});
		commitBehaviourEdit(GROUPS, 'Add content group');
	}

	/** The shortcut in the bar: an existing tag becomes a group people can switch off. */
	async function quickCreateFromTag(tag: string) {
		if (!tag) return;
		groups.push({
			id: tag,
			label: capitalize(tag),
			description: null,
			tags: [tag],
			enabled_by_default: true
		});
		quickCreateTag = '';
		commitBehaviourEdit(GROUPS, 'Add content group');
		await list?.reveal();
	}

	function removeGroup(index: number) {
		const group = groups[index];
		if (group.tags.length > 0 || group.description || group.label !== 'New group') {
			removing = group;
			return;
		}
		groups.splice(index, 1);
		commitBehaviourEdit(GROUPS, 'Remove content group');
	}

	function confirmRemove() {
		if (!removing) return;
		const index = groups.indexOf(removing);
		if (index >= 0) groups.splice(index, 1);
		removing = null;
		commitBehaviourEdit(GROUPS, 'Remove content group');
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
				value={group.label}
				placeholder="Label"
				oninput={(value) => {
					group.label = value;
					editBehaviourField(`${GROUPS}.${index}.label`, 'Edit group name');
				}}
			/>
			<Field
				label="Description (optional)"
				size="compact"
				value={group.description ?? ''}
				placeholder="Explain what this group contains"
				oninput={(value) => {
					group.description = value;
					editBehaviourField(`${GROUPS}.${index}.description`, 'Edit group description');
				}}
			/>
		</div>
		<TagPicker
			tags={group.tags}
			id={`content-group-${index}`}
			path={`${GROUPS}.${index}.tags`}
			onchange={(tags) => (group.tags = tags)}
		/>
		<label class="flex items-center gap-2">
			<Checkbox
				checked={group.enabled_by_default}
				ariaLabel="Enabled by default"
				onchange={(checked) => {
					group.enabled_by_default = checked;
					commitBehaviourEdit(`${GROUPS}.${index}.enabled_by_default`, 'Change group default');
				}}
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
