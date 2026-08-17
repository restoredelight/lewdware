<script lang="ts">
	// The subliminal pool: looping videos the mode draws over popups at low opacity -- hypno
	// spirals and the like. Not a text pool, and not a fullscreen flash: the opacity option both
	// modes already declare (`subliminal_opacity`) only means anything for something composited
	// over what is already on screen.
	//
	// Membership is the managed `__lewdware-subliminal` tag, which the editor owns and the author
	// never types (see shared/src/tags.rs). That makes this panel the only way in or out, and it
	// means the pool needs no query of its own -- `store.files` already carries every file's tags,
	// so membership is a `$derived` filter that fills in live as media streams in during an
	// import.
	//
	// There is deliberately no "choose from pack media" here. A spiral is imported to be a spiral;
	// promoting an image the pack already uses as an ordinary popup is the rare case, not the
	// common one, and offering it meant rendering a picker over every file in the pack.
	import Button from '$ui/Button.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import EmptyState from '$ui/EmptyState.svelte';
	import { Icon, Play } from 'svelte-hero-icons';
	import { api } from './api.js';
	import { history } from './history.svelte.js';
	import { openStandalonePreview } from './mediaPreview.js';
	import { store } from './store.svelte.js';
	import { NON_POPUP_TAG, SUBLIMINAL_TAG } from './tags.js';
	import { taskFeedback } from './taskFeedback.svelte.js';

	type Props = { revealId?: number | null; onrevealed?: () => void };
	let { revealId = null, onrevealed }: Props = $props();
	let poolElement = $state<HTMLDivElement>();

	const pool = $derived(store.files.filter((file) => file.tags.includes(SUBLIMINAL_TAG)));

	let removing = $state<number[] | null>(null);
	let busy = $state(false);

	$effect(() => {
		if (revealId == null || !poolElement || !pool.some((file) => file.id === revealId)) return;
		queueMicrotask(() => {
			const button = poolElement?.querySelector<HTMLButtonElement>(
				`[data-subliminal-id="${revealId}"]`
			);
			button?.focus();
			button?.scrollIntoView({ block: 'center' });
			onrevealed?.();
		});
	});

	// What leaves with the pool: a file still marked non-popup exists only to be scenery, so
	// dropping its last use drops the file too (the same rule the media slots follow). One that
	// has been un-marked is real content and stays. Counted up front because a removal that
	// quietly deletes media is not something to discover afterwards.
	const removalDeletes = $derived(
		(removing ?? []).filter(
			(id) => store.files.find((file) => file.id === id)?.tags.includes(NON_POPUP_TAG) ?? false
		).length
	);

	async function addFiles() {
		busy = true;
		try {
			const added = await api.addSubliminalFilesDialog();
			if (added === null || added.length === 0) return;
			// New imports also arrive through `upload:added`, while files that were already in the
			// pack do not. Reconcile both paths with the post-tagging values returned by the command.
			for (const file of added) store.addFile(file, true);
			history.record({
				label: added.length === 1 ? 'Add subliminal' : `Add ${added.length} subliminals`,
				storageBytes: added.reduce((total, file) => total + file.size, 0)
			});
		} catch (error) {
			taskFeedback.error('subliminals', `Could not import: ${error}`);
		} finally {
			busy = false;
		}
	}

	async function confirmRemove() {
		const ids = removing;
		if (!ids) return;
		busy = true;
		try {
			const deleted = await api.removeFromSubliminals(ids);
			store.removeTagFromFiles(ids, SUBLIMINAL_TAG, true);
			if (deleted.length > 0) store.removeFilesById(deleted, true);
			history.record({
				label: ids.length === 1 ? 'Remove subliminal' : `Remove ${ids.length} subliminals`
			});
		} catch (error) {
			taskFeedback.error('subliminals', `Could not remove from the pool: ${error}`);
		} finally {
			busy = false;
			removing = null;
		}
	}
</script>

<section class="flex flex-col gap-3" aria-label="Subliminals">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<p class="text-muted max-w-lg text-xs">
			Drawn over whatever popups are on screen, at the opacity the user chooses. Usually looping
			videos — an animated GIF counts as one once it's in a pack.
		</p>
		<div class="flex shrink-0 items-center gap-2">
			<span class="ui-metadata">{pool.length} {pool.length === 1 ? 'item' : 'items'}</span>
			<Button size="compact" disabled={busy} onclick={addFiles}>Add files…</Button>
		</div>
	</div>

	{#if pool.length === 0}
		<EmptyState
			title="No subliminals yet"
			description="Add the videos to layer over popups during a session — spirals, patterns, anything meant to sit on top rather than be looked at directly."
			actionLabel="Add files…"
			onclick={addFiles}
		/>
	{:else}
		<div class="grid grid-cols-[repeat(auto-fill,minmax(96px,1fr))] gap-2" bind:this={poolElement}>
			{#each pool as file (file.id)}
				{@const alsoPopup = !file.tags.includes(NON_POPUP_TAG)}
				<div class="tile">
					<button
						type="button"
						class="thumb"
						data-subliminal-id={file.id}
						title={`Preview ${file.file_name}`}
						onclick={() => openStandalonePreview(file.id)}
					>
						<img
							src={store.mediaUrl(`/thumbnail/${file.id}`, file.hash)}
							alt={file.file_name}
							loading="lazy"
							draggable="false"
						/>
						{#if file.file_info.type === 'video'}
							<span class="kind" aria-hidden="true"><Icon src={Play} solid /></span>
						{/if}
					</button>
					{#if alsoPopup}
						<span
							class="badge"
							title="Also spawns as an ordinary popup"
							aria-label="Also spawns as an ordinary popup"
						></span>
					{/if}
					<button
						type="button"
						class="remove"
						disabled={busy}
						aria-label={`Remove ${file.file_name} from the subliminal pool`}
						title="Remove from pool"
						onclick={() => (removing = [file.id])}
					>
						<svg viewBox="0 0 12 12" aria-hidden="true"
							><path
								d="M2 2l8 8M10 2l-8 8"
								stroke="currentColor"
								stroke-width="1.6"
								stroke-linecap="round"
							/></svg
						>
					</button>
					<span class="name" title={file.file_name}>{file.file_name}</span>
				</div>
			{/each}
		</div>
	{/if}
</section>

{#if removing}
	{@const count = removing.length}
	<Dialog
		title={count === 1 ? 'Remove from subliminals?' : `Remove ${count} from subliminals?`}
		description={removalDeletes === 0
			? 'These stay in the pack — they just stop being layered over popups.'
			: removalDeletes === count
				? `${count === 1 ? 'This was' : `All ${count} were`} only ever ${count === 1 ? 'a subliminal' : 'subliminals'}, so ${count === 1 ? 'it is' : 'they are'} deleted from the pack. The original file${count === 1 ? '' : 's'} on your computer will not be deleted.`
				: `${removalDeletes} of these ${count} were only ever subliminals and will be deleted from the pack; the rest stay, and just stop being layered over popups.`}
		buttons={[
			{ label: 'Cancel', disabled: busy, onclick: () => (removing = null) },
			{
				label: 'Remove',
				destructive: true,
				disabled: busy,
				loading: busy,
				onclick: confirmRemove
			}
		]}
		onclose={busy ? undefined : () => (removing = null)}
	/>
{/if}

<style>
	.tile {
		position: relative;
		display: flex;
		min-width: 0;
		flex-direction: column;
		align-items: center;
		gap: 4px;
	}
	.thumb {
		position: relative;
		display: flex;
		width: 100%;
		align-items: center;
		justify-content: center;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		padding: 4px;
		transition: border-color 120ms;
	}
	.thumb:hover {
		border-color: var(--ui-border-strong);
	}
	.thumb img {
		height: 72px;
		max-width: 100%;
		object-fit: contain;
	}
	/* Mirrors the media grid's video marker, since most of this pool is video. */
	.kind {
		position: absolute;
		bottom: 3px;
		left: 3px;
		display: block;
		width: 10px;
		height: 10px;
		border-radius: 2px;
		background: rgb(0 0 0 / 0.6);
		color: #fff;
	}
	.name {
		max-width: 100%;
		overflow: hidden;
		color: var(--ui-muted);
		font-size: 10px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	/* Present only when the file is *also* an ordinary popup -- rare for a spiral, which is
	   exactly why it is worth a mark when it happens. */
	.badge {
		position: absolute;
		top: 4px;
		left: 4px;
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--ui-accent-hover);
	}
	.remove {
		position: absolute;
		top: 2px;
		right: 2px;
		display: grid;
		width: 18px;
		height: 18px;
		place-items: center;
		border: 1px solid var(--ui-border-strong);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-surface);
		color: var(--ui-muted);
		opacity: 0;
		transition:
			opacity 120ms,
			color 120ms;
	}
	.remove svg {
		width: 9px;
		height: 9px;
	}
	.tile:hover .remove,
	.remove:focus-visible {
		opacity: 1;
	}
	.remove:hover {
		color: var(--ui-danger);
	}
	@media (hover: none) {
		.remove {
			opacity: 1;
		}
	}
</style>
