<script lang="ts">
	import BehaviourGate from './BehaviourGate.svelte';
	import TimelineEditor from './TimelineEditor.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import DebouncedField from './DebouncedField.svelte';
	import { keys, query } from './query.svelte.js';
	import Toggle from '$ui/Toggle.svelte';

	// The timeline is fetched whether or not it is switched on, which is the point of the `enabled`
	// flag: switching it off used to mean deleting the stages and keeping a copy in front-end
	// memory, so closing the editor lost them for good. The stages stay in the pack now.
	const timeline = query(keys.timeline, api.timeline.get);
	const enabled = $derived(timeline.current?.enabled ?? false);
	const invalidates = [keys.timeline, keys.summary];

	function enableExperience(checked: boolean) {
		const label = checked ? 'Enable timeline' : 'Disable timeline';
		// Nothing is retired: switching off drops every stage on purpose, and a cleanup driven by
		// "what stopped being referenced?" would take every stage wallpaper with it.
		void mutate(() => api.timeline.setEnabled(checked, label), { label, invalidates });
	}
</script>

<div class="flex h-full min-h-0 w-full flex-col">
	<header
		class="border-border bg-bg flex h-11 shrink-0 items-center justify-between gap-2 border-b px-3 sm:gap-4 sm:px-4"
	>
		<h2 class="text-text text-sm font-semibold">Timeline</h2>
		{#if timeline.current !== undefined}
			<div class="flex items-center gap-2">
				<span class="text-muted text-xs">{enabled ? 'Enabled' : 'Disabled'}</span>
				<Toggle ariaLabel="Enable timeline" checked={enabled} onchange={enableExperience} />
			</div>
		{/if}
	</header>
	<p class="border-border text-muted border-b px-3 py-2 text-xs sm:px-4">
		Read by the built-in modes (Sandbox and Sequence). A custom mode reads none of this.
	</p>

	<BehaviourGate title="Timeline" queries={[timeline]}>
		{#if !enabled}
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
				<DebouncedField
					value={timeline.current?.label ?? ''}
					label="Edit mode name"
					{invalidates}
					oncommit={(value: string, label: string) =>
						// A blank name means "no override" -- the mode keeps its own name.
						api.timeline.setLabel(value.trim() === '' ? null : value, label)}
				>
					{#snippet field(draft, set, commit)}
						<input
							id="mode-name"
							type="text"
							value={draft}
							placeholder="Sequence"
							oninput={(event) => set(event.currentTarget.value)}
							onblur={commit}
							class="border-border bg-surface text-text placeholder:text-muted h-8 w-56 min-w-0 rounded-sm border px-2.5 text-xs transition-colors hover:border-[var(--ui-border-strong)]"
						/>
					{/snippet}
				</DebouncedField>
			</div>
			<TimelineEditor />
		{/if}
	</BehaviourGate>
</div>
