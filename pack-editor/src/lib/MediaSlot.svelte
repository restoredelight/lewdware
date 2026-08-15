<script lang="ts">
	// One media file, addressed by a behaviour slot. The author sees the image they picked, not
	// the tag that keeps it out of popups -- the editor owns that (see shared/src/tags.rs).
	//
	// The lookup is derived from `store.files` rather than fetched: the grid already carries every
	// file with its tags, and `upload:added` appends to it, so a slot filled during an Edgeware
	// import fills in live as its file arrives.
	import Button from '$ui/Button.svelte';
	import { api } from './api.js';
	import { adoptBehaviour } from './behaviourSave.svelte.js';
	import { openStandalonePreview } from './mediaPreview.js';
	import { store } from './store.svelte.js';
	import { taskFeedback } from './taskFeedback.svelte.js';
	import type { MediaSlot } from './types.js';

	type Props = {
		slot: MediaSlot;
		/** The media id the behaviour currently says is in this slot. */
		mediaId?: number;
		title: string;
		description: string;
		/** Copy for the empty state -- what the pack loses by leaving this unset. */
		emptyNote: string;
		reveal?: boolean;
		onrevealed?: () => void;
	};

	let {
		slot,
		mediaId,
		title,
		description,
		emptyNote,
		reveal = false,
		onrevealed
	}: Props = $props();

	let busy = $state(false);
	let section = $state<HTMLElement>();

	$effect(() => {
		if (!reveal || !section) return;
		queueMicrotask(() => {
			section?.scrollIntoView({ block: 'center' });
			section?.querySelector<HTMLElement>('button')?.focus();
			onrevealed?.();
		});
	});

	const file = $derived(
		mediaId != null ? (store.files.find((f) => f.id === mediaId) ?? null) : null
	);
	// A slot pointing at a file the pack doesn't have: the import that would have brought it in
	// was cancelled, or it's still encoding. Say so rather than showing an empty frame.
	const missing = $derived(mediaId != null && file == null);

	// The backend fills the slot itself, so both handlers hand its result to `adoptBehaviour`.
	// Nothing has to be flushed first: an edit the author is still typing on another tab is
	// re-applied over the document that comes back, and would not have overwritten this slot in
	// any case -- it is sent as a patch naming only the field it touched.
	async function fill() {
		busy = true;
		try {
			const result = await api.fillMediaSlotDialog(slot);
			if (!result) return;
			// A replacement drops the file it displaced when that file was only ever this slot's --
			// the same rule, and the same bookkeeping, as clearing the slot.
			if (result.deleted_id != null) store.removeFilesById([result.deleted_id], true);
			adoptBehaviour(result.behaviour, {
				label: `Set ${title.toLowerCase()}`,
				storageBytes: result.file.size
			});
			if (!result.added) {
				taskFeedback.confirm('slot', `Already in this pack as “${result.file.file_name}”`);
			}
		} catch (error) {
			taskFeedback.error('slot', String(error));
		} finally {
			busy = false;
		}
	}

	async function clear() {
		busy = true;
		try {
			const result = await api.clearMediaSlot(slot);
			if (!result) return;
			if (result.deleted_id != null) store.removeFilesById([result.deleted_id], true);
			adoptBehaviour(result.behaviour, { label: `Clear ${title.toLowerCase()}` });
		} catch (error) {
			taskFeedback.error('slot', String(error));
		} finally {
			busy = false;
		}
	}
</script>

<section class="flex flex-col gap-2" bind:this={section}>
	<div>
		<h3 class="ui-section-title">{title}</h3>
		<p class="text-muted text-xs">{description}</p>
	</div>

	<div class="border-border bg-surface flex items-center gap-3 rounded-sm border p-2">
		{#if file}
			<button
				type="button"
				class="border-border bg-bg flex h-16 w-16 shrink-0 items-center justify-center overflow-hidden rounded-sm border transition-colors hover:border-[var(--color-border-strong)]"
				title="Preview"
				onclick={() => openStandalonePreview(file.id)}
			>
				<img
					src={store.mediaUrl(`/thumbnail/${file.id}`, file.hash)}
					alt={file.file_name}
					draggable="false"
					class="max-h-full max-w-full object-contain"
				/>
			</button>
			<div class="flex min-w-0 flex-col gap-0.5">
				<span class="text-text truncate text-sm">{file.file_name}</span>
				<span class="ui-metadata">{file.file_info.type}</span>
			</div>
		{:else}
			<div
				class="border-border text-muted flex h-16 w-16 shrink-0 items-center justify-center rounded-sm border border-dashed text-xs"
			>
				Empty
			</div>
			<p class="text-muted min-w-0 text-xs italic">
				{#if missing}
					That file isn’t in this pack any more.
				{:else}
					{emptyNote}
				{/if}
			</p>
		{/if}

		<div class="ml-auto flex shrink-0 gap-2">
			<Button size="compact" disabled={busy} onclick={fill}>
				{mediaId != null ? 'Replace…' : 'Add…'}
			</Button>
			{#if mediaId != null}
				<Button variant="destructive" size="compact" disabled={busy} onclick={clear}>Remove</Button>
			{/if}
		</div>
	</div>
</section>
