<script lang="ts">
	// One media file, addressed by a behaviour slot. The author sees the image they picked, not
	// the tag that keeps it out of popups -- the editor owns that (see shared/src/tags.rs).
	//
	// The lookup is derived from `store.files` rather than fetched: the grid already carries every
	// file with its tags, and `upload:added` appends to it, so a slot filled during an Edgeware
	// import fills in live as its file arrives.
	import { api } from './api.js';
	import { adoptBehaviour } from './behaviourSave.svelte.js';
	import ExplicitMediaPicker from './ExplicitMediaPicker.svelte';
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
		showHeader?: boolean;
	};

	let {
		slot,
		mediaId,
		title,
		description,
		emptyNote,
		reveal = false,
		onrevealed,
		showHeader = true
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

	async function selectExisting(mediaId: number) {
		busy = true;
		try {
			const result = await api.setMediaSlot(slot, mediaId);
			if (!result) return;
			if (result.deleted_id != null) store.removeFilesById([result.deleted_id], true);
			adoptBehaviour(result.behaviour, { label: `Set ${title.toLowerCase()}` });
		} catch (error) {
			taskFeedback.error('slot', String(error));
		} finally {
			busy = false;
		}
	}

	const kind = $derived<'image' | 'visual' | 'audio'>(
		slot.kind === 'stage_audio' ||
			slot.kind === 'stage_entry_sound' ||
			slot.kind === 'stage_prompt_sound'
			? 'audio'
			: slot.kind === 'splash' || slot.kind === 'stage_entry_splash'
				? 'visual'
				: 'image'
	);
</script>

<section class="flex flex-col gap-2" bind:this={section}>
	{#if showHeader}
		<div>
			<h3 class="ui-section-title">{title}</h3>
			<p class="text-muted text-xs">{description}</p>
		</div>
	{/if}

	<ExplicitMediaPicker
		{kind}
		{mediaId}
		{emptyNote}
		{busy}
		onselect={selectExisting}
		onimport={fill}
		onclear={clear}
	/>
</section>
