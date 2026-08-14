<script lang="ts">
	// One media file, addressed by a behaviour slot. The author sees the image they picked, not
	// the tag that keeps it out of popups -- the editor owns that (see shared/src/tags.rs).
	//
	// The lookup is derived from `store.files` rather than fetched: the grid already carries every
	// file with its tags, and `upload:added` appends to it, so a slot filled during an Edgeware
	// import fills in live as its file arrives.
	import Button from '$ui/Button.svelte';
	import { api } from './api.js';
	import { adoptBehaviour, flushBehaviourSave } from './behaviourSave.svelte.js';
	import { openStandalonePreview } from './mediaPreview.js';
	import { store } from './store.svelte.js';
	import { taskFeedback } from './taskFeedback.svelte.js';
	import type { MediaSlot } from './types.js';

	type Props = {
		slot: MediaSlot;
		/** What the behaviour currently says is in this slot. */
		name?: string;
		title: string;
		description: string;
		/** Copy for the empty state -- what the pack loses by leaving this unset. */
		emptyNote: string;
	};

	let { slot, name, title, description, emptyNote }: Props = $props();

	let busy = $state(false);

	const file = $derived(name ? (store.files.find((f) => f.file_name === name) ?? null) : null);
	// A slot naming a file the pack doesn't have: the import that would have brought it in was
	// cancelled, or it's still encoding. Say so rather than showing an empty frame.
	const missing = $derived(name != null && file == null);

	// The backend fills the slot itself, so both handlers flush any pending debounced write first
	// -- it holds a pre-slot document and would otherwise land afterwards and wipe the slot -- and
	// then hand the result to `adoptBehaviour`.
	async function fill() {
		busy = true;
		try {
			await flushBehaviourSave();
			const result = await api.fillMediaSlotDialog(slot);
			if (!result) return;
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
			await flushBehaviourSave();
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

<section class="flex flex-col gap-2">
	<div>
		<h3 class="ui-section-title">{title}</h3>
		<p class="text-muted text-xs">{description}</p>
	</div>

	<div class="border-border bg-surface flex items-center gap-3 rounded-sm border p-2">
		{#if file}
			<button
				type="button"
				class="border-border bg-bg hover:border-[var(--color-border-strong)] flex h-16 w-16 shrink-0 items-center justify-center overflow-hidden rounded-sm border transition-colors"
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
					“{name}” isn’t in this pack.
				{:else}
					{emptyNote}
				{/if}
			</p>
		{/if}

		<div class="ml-auto flex shrink-0 gap-2">
			<Button size="compact" disabled={busy} onclick={fill}>
				{name ? 'Replace…' : 'Add…'}
			</Button>
			{#if name}
				<Button variant="destructive" size="compact" disabled={busy} onclick={clear}>
					Remove
				</Button>
			{/if}
		</div>
	</div>
</section>
