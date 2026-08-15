<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { store } from './store.svelte.js';
	import TimelineEditor from './TimelineEditor.svelte';
	import {
		commitBehaviourEdit,
		editBehaviourField,
		ensureBehaviour,
		flushBehaviourSave
	} from './behaviourSave.svelte.js';
	import type { Experience, Stage } from './types.js';
	import Toggle from '$ui/Toggle.svelte';

	onMount(() => {
		void ensureBehaviour();
	});

	onDestroy(() => {
		flushBehaviourSave();
	});

	function emptyBaselineLevel(): Stage {
		return {
			id: crypto.randomUUID(),
			label: 'Stage 1',
			content: {},
			events: {}
		};
	}

	function enableExperience(checked: boolean) {
		if (!store.behaviour) return;
		if (checked) {
			store.behaviour.experience = store.suspendedExperience
				? structuredClone($state.snapshot(store.suspendedExperience))
				: { timeline: { stages: [emptyBaselineLevel()], transitions: [] } };
			store.suspendedExperience = null;
		} else {
			store.suspendedExperience = store.behaviour.experience
				? (structuredClone($state.snapshot(store.behaviour.experience)) as Experience)
				: null;
			store.behaviour.experience = null;
		}
		commitBehaviourEdit('experience', checked ? 'Enable timeline' : 'Disable timeline');
	}

	function setLabel(value: string) {
		if (!store.behaviour?.experience) return;
		// A blank name means "no override" -- the mode keeps its own name ("Sequence").
		store.behaviour.experience.label = value.trim() === '' ? null : value;
		editBehaviourField('experience.label', 'Edit mode name');
	}
</script>

<div class="flex h-full min-h-0 w-full flex-col">
	<header
		class="border-border bg-bg flex h-11 shrink-0 items-center justify-between gap-2 border-b px-3 sm:gap-4 sm:px-4"
	>
		<h2 class="text-text text-sm font-semibold">Timeline</h2>
		{#if store.behaviour !== null}
			<div class="flex items-center gap-2">
				<span class="text-muted text-xs">{store.behaviour.experience ? 'Enabled' : 'Disabled'}</span
				>
				<Toggle
					ariaLabel="Enable timeline"
					checked={(store.behaviour.experience && true) ?? undefined}
					onchange={enableExperience}
				/>
			</div>
		{/if}
	</header>
	<p class="border-border text-muted border-b px-3 py-2 text-xs sm:px-4">
		Read by the built-in modes (Sandbox and Sequence). A custom mode reads none of this.
	</p>

	{#if store.behaviour === null}
		<p class="text-muted p-6 text-sm">Loading…</p>
	{:else if !store.behaviour.experience}
		<div class="grid flex-1 place-items-center p-8">
			<div class="max-w-md text-center">
				<h3 class="text-text text-base font-semibold">Timeline is off</h3>
				<p class="text-muted mt-1 text-sm">
					A timeline allows you to create a more interactive experience for your pack, without
					writing your own mode.
				</p>
			</div>
		</div>
	{:else}
		<div class="border-border bg-bg flex shrink-0 items-center gap-3 border-b px-3 py-2 sm:px-4">
			<label for="mode-name" class="text-text shrink-0 text-xs font-semibold">Mode name</label>
			<input
				id="mode-name"
				type="text"
				value={store.behaviour.experience.label ?? ''}
				placeholder="Sequence"
				oninput={(event) => setLabel(event.currentTarget.value)}
				class="border-border bg-surface text-text placeholder:text-muted h-8 w-56 min-w-0 rounded-sm border px-2.5 text-xs transition-colors hover:border-[var(--ui-border-strong)]"
			/>
		</div>
		<TimelineEditor />
	{/if}
</div>
