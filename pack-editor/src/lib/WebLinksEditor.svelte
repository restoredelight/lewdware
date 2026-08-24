<script lang="ts">
	import ContentList from './ContentList.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { fields, mutate } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import type { WebLink, WebLinkRow } from './types.js';
	import EmptyState from '$ui/EmptyState.svelte';
	import Field from '$ui/Field.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import { Icon, XMark } from 'svelte-hero-icons';

	const stored = query(keys.webLinks, api.getWebLinks);
	const links = $derived(stored.current ?? []);
	const invalidates = [keys.webLinks, keys.summary];

	let newArgByLink = $state<Record<number, string>>({});
	let removing = $state<WebLinkRow | null>(null);

	/**
	 * Applies `change` to this link and sends it.
	 *
	 * Accumulated into one draft per link: the command sends the link whole, so a URL edit built
	 * from the last fetched copy would revert a suffix added a moment earlier.
	 */
	function write(
		link: WebLinkRow,
		change: (draft: WebLink) => void,
		label: string,
		debounce = false
	) {
		fields.edit<WebLink>({
			entity: `web-link:${link.id}`,
			base: () => ({ url: link.url, args: [...link.args], tags: [...link.tags] }),
			change,
			label,
			invalidates,
			send: (draft) => api.updateWebLink(link.id, draft, label),
			debounce
		});
	}

	/** What a link looks like right now: the author's unsent edit if there is one, else stored. */
	function shown(link: WebLinkRow): WebLink {
		return fields.draftFor<WebLink>(`web-link:${link.id}`) ?? link;
	}

	async function addLink() {
		await mutate(() => api.addWebLink({ url: '', args: [], tags: [] }, 'Add web link'), {
			label: 'Add web link',
			invalidates
		});
	}

	function removeLink(index: number) {
		const link = links[index];
		if (!link) return;
		if (link.url || link.args.length > 0 || link.tags.length > 0) {
			removing = link;
			return;
		}
		void mutate(() => api.removeWebLink(link.id, 'Remove web link'), {
			label: 'Remove web link',
			invalidates
		});
	}

	function confirmRemove() {
		if (!removing) return;
		const link = removing;
		removing = null;
		void mutate(() => api.removeWebLink(link.id, 'Remove web link'), {
			label: 'Remove web link',
			invalidates
		});
	}

	function addArg(index: number) {
		const link = links[index];
		const value = (newArgByLink[index] ?? '').trim();
		if (!link || !value) return;
		newArgByLink[index] = '';
		write(link, (draft) => draft.args.push(value), 'Add URL suffix');
	}

	function removeArg(linkIndex: number, argIndex: number) {
		const link = links[linkIndex];
		if (!link) return;
		write(link, (draft) => draft.args.splice(argIndex, 1), 'Remove URL suffix');
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
			value={shown(link).url}
			placeholder="https://…"
			oninput={(value) => write(link, (draft) => (draft.url = value), 'Edit web link', true)}
		/>

		<div>
			<p class="text-muted mb-1.5 text-xs font-medium">
				Random URL suffixes <span class="font-normal">(optional)</span>
			</p>
			<div class="flex flex-wrap items-center gap-1.5">
				{#each shown(link).args as arg, argIndex}
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
			tags={shown(link).tags}
			id={`web-link-${index}`}
			onchange={(tags, label) => write(link, (draft) => (draft.tags = tags), label)}
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
