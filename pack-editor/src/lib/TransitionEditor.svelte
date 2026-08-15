<script lang="ts">
	import Checkbox from '$ui/Checkbox.svelte';
	import Select from '$ui/Select.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import { store } from './store.svelte.js';
	import type { Stage, Transition, TransitionValue } from './types.js';

	type Props = { transitionId: string; from: Stage; to: Stage; onstage: (id: string) => void };
	let { transitionId, from, to, onstage }: Props = $props();
	// Looked up from the store rather than received as a prop: this editor mutates the
	// transition, and mutating an unbound prop trips Svelte's ownership warning.
	const transition = $derived(
		store.behaviour!.experience!.timeline.transitions.find((item) => item.id === transitionId)!
	);
	// A patch addresses a transition by position, while everything else here addresses it by id --
	// the ids are what survive the timeline being edited, but the document is a list.
	const path = $derived(
		`experience.timeline.transitions.${store.behaviour!.experience!.timeline.transitions.findIndex(
			(item) => item.id === transitionId
		)}`
	);
	let rootEl = $state<HTMLElement>();
	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// so reset the scroll when this editor switches to a different transition.
	$effect(() => {
		transitionId;
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
				{ key: 'subliminal_interval', label: 'Subliminals' }
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
	function isAffected(group: (typeof groups)[number], key: TransitionValue) {
		return transition.affected.includes(group.legacy) || transition.affected.includes(key);
	}
	function affected(group: (typeof groups)[number], key: TransitionValue, checked: boolean) {
		// Expanding a legacy broad selection on first edit keeps every sibling selected.
		if (transition.affected.includes(group.legacy)) {
			transition.affected = transition.affected.filter((item) => item !== group.legacy);
			for (const value of group.values)
				if (!transition.affected.includes(value.key)) transition.affected.push(value.key);
		}
		if (checked && !transition.affected.includes(key)) transition.affected.push(key);
		if (!checked) transition.affected = transition.affected.filter((item) => item !== key);
		// Named for the section it lives in ("Gradual changes"), so the undo entry points at
		// something the author can see rather than describing the field.
		commitBehaviourEdit(`${path}.affected`, 'Edit gradual changes');
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
				<label
					>Duration (seconds)<input
						type="number"
						min="0"
						step="1"
						value={transition.duration_seconds}
						oninput={(event) => {
							const value = event.currentTarget.valueAsNumber;
							if (Number.isFinite(value)) {
								transition.duration_seconds = value;
								editBehaviourField(`${path}.duration_seconds`, 'Edit transition duration');
							}
						}}
					/></label
				>
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
					onchange={(value) => {
						transition.easing = value as Transition['easing'];
						commitBehaviourEdit(`${path}.easing`, 'Change transition easing');
					}}
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
	.fields > label {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 5px;
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 600;
	}
	.fields input {
		width: 100%;
		height: 36px;
		padding: 0 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
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
