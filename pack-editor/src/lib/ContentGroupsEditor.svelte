<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	import Select from '$ui/Select.svelte';
	import { store } from './store.svelte.js';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { Icon, Plus } from 'svelte-hero-icons';
	import { tick } from 'svelte';

	const groups = $derived(store.behaviour!.content.content_groups);
	// Adding or removing a group moves every later index, so those edits replace the array whole
	// rather than addressing one entry.
	const GROUPS = 'content.content_groups';

	let quickCreateTag = $state('');
	let removing = $state<(typeof groups)[number] | null>(null);
	let listElement = $state<HTMLDivElement>();

	function capitalize(s: string): string {
		return s.length === 0 ? s : s[0].toUpperCase() + s.slice(1);
	}

	async function revealNewGroup() {
		await tick();
		const group = listElement?.lastElementChild;
		group?.scrollIntoView({ block: 'center' });
		group?.querySelector<HTMLInputElement>('input')?.focus();
	}

	async function addGroup() {
		groups.push({
			id: `group-${Date.now()}`,
			label: 'New group',
			description: null,
			tags: [],
			enabled_by_default: true
		});
		commitBehaviourEdit(GROUPS, 'Add content group');
		await revealNewGroup();
	}

	async function quickCreateFromTag() {
		const tag = quickCreateTag;
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
		await revealNewGroup();
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

<section class="flex flex-col gap-3" aria-label="Content groups">
	{#if groups.length > 0}
		<div
			class="border-border bg-bg sticky top-0 z-10 flex items-center justify-between gap-3 border-y py-2"
		>
			<span class="ui-metadata">{groups.length} {groups.length === 1 ? 'item' : 'items'}</span>
			<Button size="compact" onclick={addGroup}
				><span class="h-4 w-4"><Icon src={Plus} mini /></span> Add group</Button
			>
		</div>
	{/if}

	<div class="flex flex-col gap-2" bind:this={listElement}>
		{#if groups.length === 0}
			<EmptyState
				title="No content groups yet"
				description="Create a group when you want people to opt in or out of related tagged content."
				actionLabel="Add content group"
				onclick={addGroup}
			/>
		{/if}
		{#each groups as group, index}
			<Card class="flex flex-col gap-3 p-3">
				<div class="flex items-center justify-between">
					<span class="text-muted font-mono text-[11px] font-semibold">Group {index + 1}</span
					><Button
						size="compact"
						variant="destructive"
						class="!h-7"
						onclick={() => removeGroup(index)}>Remove</Button
					>
				</div>
				<div class="flex items-start gap-2">
					<div class="flex flex-1 flex-col gap-1.5">
						<label class="text-muted text-xs font-medium" for={`group-name-${index}`}
							>Group name</label
						>
						<input
							id={`group-name-${index}`}
							bind:value={group.label}
							oninput={() => editBehaviourField(`${GROUPS}.${index}.label`, 'Edit group name')}
							placeholder="Label"
							class="border-border bg-bg text-text rounded border px-2 py-1 text-xs
"
						/>
						<label class="text-muted mt-1 text-xs font-medium" for={`group-description-${index}`}
							>Description <span class="font-normal">(optional)</span></label
						>
						<input
							id={`group-description-${index}`}
							bind:value={group.description}
							oninput={() =>
								editBehaviourField(`${GROUPS}.${index}.description`, 'Edit group description')}
							placeholder="Explain what this group contains"
							class="border-border bg-bg text-text rounded border px-2 py-1 text-xs
"
						/>
					</div>
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
			</Card>
		{/each}
	</div>

	<div class="flex items-center gap-2">
		{#if groups.length > 0}<Button size="compact" onclick={addGroup}
				><span class="h-4 w-4"><Icon src={Plus} mini /></span> Add group</Button
			>{/if}

		{#if store.allTags.length > 0}
			<span class="text-muted text-xs">or</span>
			<Select
				class="w-48"
				size="compact"
				hideLabel
				label="Tag to make toggleable"
				value={quickCreateTag}
				options={[
					{ value: '', label: 'Make a tag toggleable…' },
					...store.allTags.map((tag) => ({ value: tag, label: tag }))
				]}
				onchange={(value) => (quickCreateTag = value)}
			/>
			<Button size="compact" onclick={quickCreateFromTag} disabled={!quickCreateTag}>Create</Button>
		{/if}
	</div>
</section>

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
