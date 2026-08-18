<script lang="ts">
	import { store } from './store.svelte.js';
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Field from '$ui/Field.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { Icon, XMark } from 'svelte-hero-icons';

	const links = $derived(store.behaviour!.content.web_links);
	// Adding or removing a link moves every later index, so those edits replace the array whole
	// rather than addressing one entry.
	const LINKS = 'content.web_links';

	let newArgByLink = $state<Record<number, string>>({});
	let removing = $state<(typeof links)[number] | null>(null);

	function addLink() {
		links.push({ url: '', args: [], tags: [] });
		commitBehaviourEdit(LINKS, 'Add web link');
	}

	function removeLink(index: number) {
		const link = links[index];
		if (link.url || link.args.length > 0 || link.tags.length > 0) {
			removing = link;
			return;
		}
		links.splice(index, 1);
		commitBehaviourEdit(LINKS, 'Remove web link');
	}

	function confirmRemove() {
		if (!removing) return;
		const index = links.indexOf(removing);
		if (index >= 0) links.splice(index, 1);
		removing = null;
		commitBehaviourEdit(LINKS, 'Remove web link');
	}

	function addArg(index: number) {
		const value = (newArgByLink[index] ?? '').trim();
		if (!value) return;
		links[index].args.push(value);
		newArgByLink[index] = '';
		commitBehaviourEdit(`${LINKS}.${index}.args`, 'Add URL suffix');
	}

	function removeArg(linkIndex: number, argIndex: number) {
		links[linkIndex].args.splice(argIndex, 1);
		commitBehaviourEdit(`${LINKS}.${linkIndex}.args`, 'Remove URL suffix');
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
	{#snippet intro()}
		<p class="text-muted text-xs">
			Optional URL suffixes can be appended at random—for example, to choose from several search
			terms. Leave them empty to always open the URL unchanged.
		</p>
	{/snippet}
	{#snippet empty()}
		<EmptyState
			title="No web links yet"
			description="Add a link if this pack should be able to open a page in the user’s browser."
			actionLabel="Add web link"
			onclick={addLink}
		/>
	{/snippet}
	{#snippet fields(link, index)}
		<Field
			label="URL"
			type="url"
			size="compact"
			value={link.url}
			placeholder="https://…"
			oninput={(value) => {
				link.url = value;
				editBehaviourField(`${LINKS}.${index}.url`, 'Edit web link');
			}}
		/>

		<div>
			<p class="text-muted mb-1.5 text-xs font-medium">
				Random URL suffixes <span class="font-normal">(optional)</span>
			</p>
			<div class="flex flex-wrap items-center gap-1.5">
				{#each link.args as arg, argIndex}
					<span
						class="bg-bg border-border text-text flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs"
					>
						{arg}
						<button
							onclick={() => removeArg(index, argIndex)}
							class="text-muted hover:text-text leading-none"
							aria-label={`Remove URL suffix ${arg}`}
							><span class="block h-3.5 w-3.5"><Icon src={XMark} mini /></span></button
						>
					</span>
				{/each}
				<input
					value={newArgByLink[index] ?? ''}
					oninput={(event) => (newArgByLink[index] = event.currentTarget.value)}
					onkeydown={(event) => {
						if (event.key === 'Enter') {
							event.preventDefault();
							addArg(index);
						}
					}}
					placeholder="Add arg…"
					class="border-border bg-surface text-text w-24 rounded border px-2 py-0.5 text-xs"
				/>
			</div>
		</div>

		<TagPicker
			tags={link.tags}
			id={`web-link-${index}`}
			path={`${LINKS}.${index}.tags`}
			onchange={(tags) => (link.tags = tags)}
		/>
	{/snippet}
</ContentList>

{#if removing}
	<Dialog
		title="Remove web link?"
		description="This link, its URL suffixes, and its tag assignments will be removed. You can undo this change from the editor history."
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			{ label: 'Remove link', destructive: true, onclick: confirmRemove }
		]}
		onclose={() => (removing = null)}
	/>
{/if}
