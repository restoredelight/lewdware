<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import Select from '$ui/Select.svelte';
	import { api } from './api.js';
	import { mutate } from './mutate.svelte.js';
	import DebouncedField from './DebouncedField.svelte';
	import { keys } from './query.svelte.js';
	import type { Stage, Transition, TransitionValue } from './types.js';

	// The transition comes down as a prop now that nothing here mutates it in place: an edit builds
	// a draft and sends it, so the old reason for looking it up out of a shared document (mutating
	// an unbound prop trips Svelte's ownership warning) is gone with the mutation.
	type Props = { transition: Transition; from: Stage; to: Stage; onstage: (id: string) => void };
	let { transition, from, to, onstage }: Props = $props();

	// Addressed by id in the write: a transition's position is a fact about the timeline at the
	// moment this rendered, and the id is what survives editing it.
	const invalidates = [keys.timeline];

	/** Sends one field of this transition. Nothing else about it travels with the change. */
	function write(run: () => Promise<void>, label: string) {
		void mutate(run, { label, invalidates });
	}

	let rootEl = $state<HTMLElement>();
	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// so reset the scroll when this editor switches to a different transition.
	$effect(() => {
		transition.id;
		rootEl?.scrollTo(0, 0);
	});
	const groups: {
		label: string;
		legacy: TransitionValue;
		values: { key: TransitionValue; label: string }[];
	}[] = [
		{
			label: 'Event intervals',
			legacy: 'events',
			values: [
				{ key: 'popup_interval', label: 'Popups' },
				{ key: 'web_interval', label: 'Web links' },
				{ key: 'notification_interval', label: 'Notifications' },
				{ key: 'prompt_interval', label: 'Prompts' },
				{ key: 'sound_interval', label: 'Sounds' }
			]
		},
		{
			label: 'Movement',
			legacy: 'movement',
			values: [
				{ key: 'movement_minimum_speed', label: 'Minimum speed' },
				{ key: 'movement_maximum_speed', label: 'Maximum speed' }
			]
		},
		{
			label: 'Mitosis',
			legacy: 'mitosis',
			values: [
				{ key: 'mitosis_chance', label: 'Chance' },
				{ key: 'mitosis_count', label: 'Number of copies' }
			]
		}
	];
	const crossfadeGroup = {
		label: 'Audio',
		legacy: 'crossfade' as TransitionValue,
		values: [{ key: 'crossfade' as TransitionValue, label: 'Background audio' }]
	};
	function isAffected(group: (typeof groups)[number], key: TransitionValue) {
		return transition.affected.includes(group.legacy) || transition.affected.includes(key);
	}
	function affected(_group: (typeof groups)[number], key: TransitionValue, checked: boolean) {
		// One checkbox, one command. Building the whole list here and sending that would mean two
		// quick clicks each dropped the other's work — and the legacy broad-category expansion the
		// backend does is the same edit, so it belongs in the same place.
		//
		// Named for the section it lives in ("Gradual changes"), so the undo entry points at
		// something the author can see rather than describing the field.
		write(
			() => api.transition.setCategory(transition.id, key, checked, 'Edit gradual changes'),
			'Edit gradual changes'
		);
	}
</script>

<main class="transition-editor" bind:this={rootEl}>
	<div class="panel">
		<header>
			<div>
				<span class="eyebrow">Transition</span>
				<h2>
					<button onclick={() => onstage(from.id)}>{from.label}</button><span>to</span><button
						onclick={() => onstage(to.id)}>{to.label}</button
					>
				</h2>
				<p>Choose how values change after {from.label} has finished.</p>
			</div>
		</header>
		<section class="card">
			<div class="section-title">
				<div>
					<h3>Timing</h3>
					<p>A zero-second transition changes everything immediately.</p>
				</div>
			</div>
			<div class="fields">
				<DebouncedField
					value={transition.duration_seconds}
					label="Edit transition duration"
					{invalidates}
					oncommit={(seconds: number, label: string) =>
						api.transition.setDuration(transition.id, seconds, label)}
				>
					{#snippet field(draft, set, commit)}
						<NumberField
							label="Duration (seconds)"
							min={0}
							step={1}
							value={draft}
							oninput={(seconds) => {
								// An empty field is mid-edit, not a request for an instant transition:
								// leave the value where it is until a real number is typed.
								if (seconds !== null) set(seconds);
							}}
							onchange={() => commit()}
						/>
					{/snippet}
				</DebouncedField>
				<Select
					label="Easing"
					value={transition.easing}
					disabled={transition.duration_seconds === 0}
					options={[
						{ value: 'linear', label: 'Linear' },
						{ value: 'ease_in', label: 'Ease in' },
						{ value: 'ease_out', label: 'Ease out' },
						{ value: 'ease_in_out', label: 'Ease in and out' }
					]}
					onchange={(value) =>
						write(
							() =>
								api.transition.setEasing(
									transition.id,
									value as Transition['easing'],
									'Change transition easing'
								),
							'Change transition easing'
						)}
				/>
			</div>
		</section>
		<section class="card" class:disabled={transition.duration_seconds === 0}>
			<div class="section-title">
				<div>
					<h3>Gradual changes</h3>
					<p>
						Choose exactly which numeric values interpolate. All other changes take effect at the
						end.
					</p>
				</div>
			</div>
			{#each groups as group}
				<div class="group">
					<div class="group-label">{group.label}</div>
					<div class="value-grid">
						{#each group.values as value}
							<label class="value"
								><Checkbox
									checked={isAffected(group, value.key)}
									disabled={transition.duration_seconds === 0}
									ariaLabel={`Gradually change ${value.label}`}
									onchange={(checked) => affected(group, value.key, checked)}
								/><span>{value.label}</span></label
							>
						{/each}
					</div>
				</div>
			{/each}
			<div class="group">
				<div class="group-label">Audio</div>
				<div class="value-grid">
					<label class="value"
						><Checkbox
							checked={transition.affected.includes('crossfade')}
							disabled={transition.duration_seconds === 0}
							ariaLabel="Crossfade background audio"
							onchange={(checked) => affected(crossfadeGroup, 'crossfade', checked)}
						/><span>Crossfade background audio</span></label
					>
				</div>
			</div>
		</section>
		<p class="end-note">
			Content selections and enabled or disabled features switch to {to.label}’s values when this
			transition ends.
		</p>
	</div>
</main>

<style>
	.transition-editor {
		flex: 1;
		min-width: 0;
		padding: 24px;
		overflow-y: auto;
	}
	.panel {
		display: flex;
		width: 100%;
		max-width: 800px;
		min-width: 0;
		margin-inline: auto;
		flex-direction: column;
		gap: 14px;
	}
	.eyebrow {
		color: var(--ui-muted);
		font-family: var(--ui-font-mono);
		font-size: 11px;
		font-weight: 700;
	}
	h2 {
		display: flex;
		margin: 3px 0 0;
		align-items: center;
		gap: 8px;
		font-size: 17px;
		font-weight: 650;
	}
	h2 span {
		color: var(--ui-muted);
		font-size: 13px;
		font-weight: 400;
	}
	h2 button {
		padding: 0;
		border: 0;
		background: transparent;
		color: var(--ui-text);
		font: inherit;
		text-decoration: underline;
		text-decoration-color: var(--ui-border-strong);
		text-underline-offset: 3px;
		cursor: pointer;
	}
	header p,
	.section-title p {
		margin: 4px 0 0;
		color: var(--ui-muted);
		font-size: 12px;
	}
	.card {
		display: flex;
		padding: 16px;
		flex-direction: column;
		gap: 12px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-md);
		background: var(--ui-surface);
	}
	.card.disabled {
		opacity: 0.65;
	}
	.section-title h3 {
		margin: 0;
		font-size: 16px;
	}
	.fields {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		align-items: end;
		gap: 12px;
	}
	.group {
		min-width: 0;
		padding-top: 12px;
		border-top: 1px solid var(--ui-border);
	}
	.group-label {
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 650;
	}
	.value-grid {
		display: grid;
		margin-top: 9px;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 9px 16px;
	}
	.value {
		display: flex;
		min-width: 0;
		align-items: center;
		gap: 9px;
		color: var(--ui-text);
		font-size: 12px;
		cursor: pointer;
	}
	.end-note {
		margin: 0;
		padding: 0 4px;
		color: var(--ui-muted);
		font-size: 11px;
	}
	@media (max-width: 700px) {
		.transition-editor {
			padding: 16px;
		}
		.fields,
		.value-grid {
			grid-template-columns: 1fr;
		}
	}
</style>
