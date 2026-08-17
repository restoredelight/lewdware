<script lang="ts">
	import { store } from './store.svelte.js';
	import TagPicker from './TagPicker.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { Icon, Plus, XMark } from 'svelte-hero-icons';
	import { tick } from 'svelte';

	const links = $derived(store.behaviour!.content.web_links);
	// Adding or removing a link moves every later index, so those edits replace the array whole
	// rather than addressing one entry.
	const LINKS = 'content.web_links';

	let newArgByLink = $state<Record<number, string>>({});
	let removing = $state<(typeof links)[number] | null>(null);
	let listElement = $state<HTMLDivElement>();

	async function addLink() {
		links.push({ url: '', args: [], tags: [] });
		commitBehaviourEdit(LINKS, 'Add web link');
		await tick();
		const link = listElement?.lastElementChild;
		link?.scrollIntoView({ block: 'center' });
		link?.querySelector<HTMLInputElement>('input')?.focus();
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

<section class="flex flex-col gap-3" aria-label="Web links">
	<p class="text-muted text-xs">
		Optional URL suffixes can be appended at random—for example, to choose from several search
		terms. Leave them empty to always open the URL unchanged.
	</p>

	{#if links.length > 0}
		<div
			class="border-border bg-bg sticky top-0 z-10 flex items-center justify-between gap-3 border-y py-2"
		>
			<span class="ui-metadata">{links.length} {links.length === 1 ? 'item' : 'items'}</span>
			<Button size="compact" onclick={addLink}
				><span class="h-4 w-4"><Icon src={Plus} mini /></span> Add web link</Button
			>
		</div>
	{/if}

	<div class="flex flex-col gap-2" bind:this={listElement}>
		{#if links.length === 0}
			<EmptyState
				title="No web links yet"
				description="Add a link if this pack should be able to open a page in the user’s browser."
				actionLabel="Add web link"
				onclick={addLink}
			/>
		{/if}
		{#each links as link, index}
			<Card class="flex flex-col gap-3 p-3">
				<div class="flex items-center justify-between">
					<span class="text-muted font-mono text-[11px] font-semibold">Web link {index + 1}</span
					><Button
						size="compact"
						variant="destructive"
						class="!h-7"
						onclick={() => removeLink(index)}>Remove</Button
					>
				</div>
				<label class="text-muted text-xs font-medium" for={`web-link-url-${index}`}>URL</label>
				<div class="flex items-start gap-2">
					<input
						id={`web-link-url-${index}`}
						bind:value={link.url}
						oninput={() => editBehaviourField(`${LINKS}.${index}.url`, 'Edit web link')}
						placeholder="https://…"
						class="border-border bg-bg text-text flex-1 rounded border px-2 py-1 text-xs
"
					/>
				</div>

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
									aria-label="Remove arg"
									><span class="block h-3.5 w-3.5"><Icon src={XMark} mini /></span></button
								>
							</span>
						{/each}
						<input
							value={newArgByLink[index] ?? ''}
							oninput={(e) => (newArgByLink[index] = e.currentTarget.value)}
							onkeydown={(e) => {
								if (e.key === 'Enter') {
									e.preventDefault();
									addArg(index);
								}
							}}
							placeholder="Add arg…"
							class="border-border bg-surface text-text w-24 rounded border px-2 py-0.5 text-xs
"
						/>
					</div>
				</div>

				<TagPicker
					tags={link.tags}
					id={`web-link-${index}`}
					path={`${LINKS}.${index}.tags`}
					onchange={(tags) => (link.tags = tags)}
				/>
			</Card>
		{/each}
	</div>

	{#if links.length > 0}<Button size="compact" class="self-start" onclick={addLink}
			><span class="h-4 w-4"><Icon src={Plus} mini /></span> Add web link</Button
		>{/if}
</section>

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
