<script lang="ts">
	import { Icon, Plus } from 'svelte-hero-icons';
	import Button from '$ui/Button.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Select from '$ui/Select.svelte';
	import Toggle from '$ui/Toggle.svelte';
	import EventScheduleEditor from './EventScheduleEditor.svelte';
	import MediaSlot from './MediaSlot.svelte';
	import StageTabs from './StageTabs.svelte';
	import TagPicker from './TagPicker.svelte';
	import TransitionEditor from './TransitionEditor.svelte';
	import { commitBehaviourEdit, editBehaviourField } from './behaviourSave.svelte.js';
	import { stageTagName, tagClaims, takenTagNames } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { EventSchedule, Stage, TagAction } from './types.js';
	import {
		duplicateStage as duplicateTimelineStage,
		moveStage as moveTimelineStage,
		normalizeTimeline,
		removeStage as removeTimelineStage
	} from './timelineModel.js';

	const stages = $derived(store.behaviour!.experience!.timeline.stages);
	const transitions = $derived(store.behaviour!.experience!.timeline.transitions);
	let activeId = $state('');
	let removing = $state<Stage | null>(null);
	let mainEl = $state<HTMLElement>();
	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// leaving a shorter stage blank and unscrollable.
	$effect(() => {
		activeId;
		mainEl?.scrollTo(0, 0);
	});
	$effect(() => {
		const target = store.experienceTargetStageId;
		if (target && stages.some((item) => item.id === target)) activeId = target;
	});
	$effect(() => {
		if (
			(!activeId || ![...stages, ...transitions].some((item) => item.id === activeId)) &&
			stages[0]
		)
			activeId = stages[0].id;
	});
	const activeIndex = $derived(stages.findIndex((stage) => stage.id === activeId));
	const stage = $derived(activeIndex >= 0 ? stages[activeIndex] : undefined);
	const transition = $derived(transitions.find((item) => item.id === activeId));
	const transitionFrom = $derived(
		transition ? stages.find((item) => item.id === transition.from_stage) : undefined
	);
	const transitionTo = $derived(
		transition ? stages.find((item) => item.id === transition.to_stage) : undefined
	);
	const previous = $derived(activeIndex > 0 ? stages[activeIndex - 1] : undefined);
	// What this stage's wallpaper would be if it sets none: the nearest earlier stage that sets
	// one, or the pack's own. A stage's wallpaper is an absolute write, so "empty" isn't "none" --
	// it's "whatever is already up", and the author can only judge that if we name it.
	const inheritedWallpaper = $derived.by(() => {
		for (let index = activeIndex - 1; index >= 0; index--) {
			const name = stages[index].content.wallpaper;
			if (name) return name;
		}
		return store.behaviour?.content.wallpaper;
	});
	const inheritedAudio = $derived.by(() => {
		for (let index = activeIndex - 1; index >= 0; index--) {
			const media = stages[index].content.audio;
			if (media) return media;
		}
		return undefined;
	});
	const next = $derived(activeIndex >= 0 ? stages[activeIndex + 1] : undefined);
	const outgoing = $derived(
		stage && next
			? transitions.find((item) => item.from_stage === stage.id && item.to_stage === next.id)
			: undefined
	);
	const eventDefs = [
		{ key: 'popup', label: 'Popups', interval: 30 },
		{ key: 'web', label: 'Web links', interval: 300 },
		{ key: 'notification', label: 'Notifications', interval: 300 },
		{ key: 'prompt', label: 'Prompts', interval: 90 },
		{ key: 'subliminal', label: 'Subliminals', interval: 60 },
		{ key: 'sound', label: 'Sounds', interval: 90 }
	] as const;
	const clone = <T,>(value: T): T => structuredClone($state.snapshot(value)) as T;
	// Adding, moving or removing a stage renumbers the rest and rewrites the transitions between
	// them (`normalizeTimeline`), so those edits address the timeline whole. Editing one stage's
	// own fields addresses just that stage.
	const TIMELINE = 'experience.timeline';
	const stagePath = $derived(`${TIMELINE}.stages.${activeIndex}`);
	function id(prefix: string) {
		return `${prefix}-${crypto.randomUUID()}`;
	}
	function changed(label: string) {
		normalizeTimeline(store.behaviour!.experience!.timeline);
		commitBehaviourEdit(TIMELINE, label);
	}
	function addStage() {
		// With a transition selected, insert between its two stages; otherwise after the active stage.
		const source = stage ?? transitionFrom ?? stages[stages.length - 1];
		let insertIndex = stages.length;
		if (stage) insertIndex = activeIndex + 1;
		else if (transition) {
			const toIndex = stages.findIndex((item) => item.id === transition.to_stage);
			if (toIndex >= 0) insertIndex = toIndex;
		}
		const next: Stage = source ? clone(source) : { id: '', label: '', content: {}, events: {} };
		next.id = id('stage');
		next.label = `Stage ${stages.length + 1}`;
		// Same rule as duplicating: the new stage inherits the source's selection but owns none of
		// it, so renaming it cannot rewrite a tag the source is reading. See `timelineModel.ts`.
		delete next.content.owned_tag;
		stages.splice(insertIndex, 0, next);
		activeId = next.id;
		changed('Add stage');
	}
	function duplicate(index = activeIndex) {
		const source = stages[index];
		if (!source) return;
		const snapshot = $state.snapshot(source) as Stage;
		const next = duplicateTimelineStage(store.behaviour!.experience!.timeline, index, snapshot);
		activeId = next.id;
		commitBehaviourEdit(TIMELINE, 'Duplicate stage');
	}
	function move(from: number, to: number) {
		const selected = stages[from];
		moveTimelineStage(store.behaviour!.experience!.timeline, from, to - from);
		activeId = selected.id;
		commitBehaviourEdit(TIMELINE, 'Move stage');
	}
	// ── The stage's own tag ────────────────────────────────────────────────────
	//
	// A stage that restricts its content needs a tag for the "Appears in" strip to write, and
	// leaving authors to invent one per stage gives the pack arbitrary names that mean nothing to
	// anyone reading it. So the editor creates one, keeps its name in step with the stage's, and
	// takes it away again when the stage goes and nothing else is holding it. Naming and the
	// claim check live in `stageTags.ts`; this is where the three moments are.

	/** Every tag name the pack already has, so a new one cannot land on an existing classification. */
	function taken(except?: string) {
		const names = takenTagNames(store.behaviour, store.allTags);
		if (except) names.delete(except);
		return names;
	}

	/**
	 * Turns a stage's tag selection on or off.
	 *
	 * Switching it on is the cliff: the stage goes from *everything* to *nothing* in one checkbox,
	 * because an empty inclusion list selects no media. So the stage's new tag is seeded onto the
	 * media that appears there right now — which, for a stage that restricted nothing, is the whole
	 * pack. Behaviour is preserved exactly, and only then do the per-file toggles have a tag to
	 * remove. See `behaviour-design/default-mode-v2.md`, "Turning a stage's tags on is the cliff".
	 */
	function setRestriction(on: boolean) {
		if (!stage) return;
		if (!on) {
			delete stage.content.tags;
			delete stage.content.owned_tag;
			commitBehaviourEdit(`${stagePath}.content`, 'Change stage content');
			return;
		}
		const tag = stageTagName(stage.label, taken());
		stage.content.tags = [tag];
		stage.content.owned_tag = tag;
		commitBehaviourEdit(
			`${stagePath}.content`,
			'Restrict stage content',
			[],
			[{ kind: 'apply', tag, media: null }]
		);
		store.addTagToFiles(
			store.files.map((file) => file.id),
			tag,
			true
		);
	}

	/**
	 * Follows a stage rename through to the tag it owns, on blur rather than on every keystroke —
	 * `stage-i`, `stage-in`, `stage-int` are renames of a real tag, not an intermediate state.
	 *
	 * Only a tag no other stage selects by. A rename is lossless everywhere else it appears (the
	 * lists are joins, so media, pools and groups follow the id), but another stage reading this
	 * name is another author decision, and this one is bookkeeping.
	 */
	function renameOwnedTag() {
		const owned = stage?.content.owned_tag;
		if (!stage || !owned) return;
		if (stages.some((other) => other.id !== stage.id && (other.content.tags ?? []).includes(owned)))
			return;
		const next = stageTagName(stage.label, taken(owned));
		if (next === owned) return;
		stage.content.tags = (stage.content.tags ?? []).map((tag) => (tag === owned ? next : tag));
		stage.content.owned_tag = next;
		// Same label as the field's own edits, so the rename and the typing that caused it coalesce
		// into one undo entry rather than two.
		commitBehaviourEdit(stagePath, 'Rename stage', [], [{ kind: 'rename', from: owned, to: next }]);
	}

	/** What still holds the stage being removed's tag, for the confirmation to say out loud. */
	const removingClaims = $derived(
		removing?.content.owned_tag
			? tagClaims(store.behaviour, removing.content.owned_tag, removing.id, store.files)
			: null
	);

	function removalNote() {
		const tag = removing?.content.owned_tag;
		if (!tag || !removingClaims) return '';
		if (!removingClaims.claimed) return ` Its tag “${tag}” is unused and goes with it.`;
		const parts: string[] = [];
		const { media, stages: others, content } = removingClaims;
		if (media > 0) parts.push(`${media} file${media === 1 ? '' : 's'}`);
		if (others.length > 0)
			parts.push(others.length === 1 ? `“${others[0]}”` : `${others.length} other stages`);
		if (content > 0) parts.push(`${content} content ${content === 1 ? 'entry' : 'entries'}`);
		return ` Its tag “${tag}” is on ${parts.join(', ')}, and is kept unless you remove it too.`;
	}

	function confirmRemove(alsoRemoveTag = false) {
		if (!removing || stages.length === 1) return;
		const target = removing;
		const index = stages.indexOf(target);
		// A stage's wallpaper is a media slot, so retiring the stage retires the slot: a file that
		// was only ever this stage's scenery leaves with it, exactly as the slot's own Remove does
		// (see `MediaPack::clear_media_slot`). Handed to the backend as `retiring` rather than
		// cleared through a command of its own, so that dropping the stage and dropping its
		// wallpaper are one transaction -- and so one undo brings back both, instead of leaving a
		// stage without the wallpaper it had.
		const retiring = [target.content.wallpaper, target.content.audio].filter(
			(value): value is number => value != null
		);
		// And the same for the tag the stage owns, for the same reason and in the same transaction.
		// Unconditional removal only where the author asked for it having been told what it is on;
		// otherwise the backend drops it if — and only if — nothing turns out to claim it.
		const owned = target.content.owned_tag;
		const tagActions: TagAction[] = owned
			? [
					alsoRemoveTag
						? { kind: 'delete', tag: owned }
						: { kind: 'retire_if_unclaimed', tag: owned }
				]
			: [];
		removeTimelineStage(store.behaviour!.experience!.timeline, target);
		activeId = stages[Math.min(index, stages.length - 1)].id;
		removing = null;
		commitBehaviourEdit(TIMELINE, 'Remove stage', retiring, tagActions);
	}
	function eventValue(key: keyof Stage['events']): EventSchedule | undefined {
		return stage?.events[key];
	}
	function setEvent(key: keyof Stage['events'], value?: EventSchedule) {
		if (!stage) return;
		if (value) stage.events[key] = value;
		else delete stage.events[key];
		commitBehaviourEdit(`${stagePath}.events`, 'Change stage events');
	}
	function setEntryEffect(key: 'splash' | 'sound' | 'notification', on: boolean) {
		if (!stage) return;
		stage.on_enter ??= {};
		if (on) stage.on_enter[key] = true;
		else delete stage.on_enter[key];
		if (
			!stage.on_enter.splash &&
			!stage.on_enter.sound &&
			!stage.on_enter.notification &&
			!stage.on_enter.popup_burst
		)
			delete stage.on_enter;
		commitBehaviourEdit(`${stagePath}.on_enter`, 'Change stage entry effects');
	}
	function transitionSummary() {
		if (!outgoing || outgoing.duration_seconds === 0) return 'Immediately';
		const minutes = outgoing.duration_seconds / 60;
		return `Gradually over ${Number.isInteger(minutes) ? `${minutes} minute${minutes === 1 ? '' : 's'}` : `${outgoing.duration_seconds} seconds`}`;
	}
</script>

<section class="layout">
	<aside>
		<div class="tabs">
			<StageTabs
				{stages}
				{transitions}
				active={activeId}
				onselect={(id) => (activeId = id)}
				onmove={move}
				onduplicate={duplicate}
				ondelete={(item) => (removing = item)}
			/>
		</div>
		<Button
			size="compact"
			class="px-auto w-full max-[700px]:w-auto max-[700px]:shrink-0"
			onclick={addStage}><Icon src={Plus} mini width="auto" height="25px" />Add stage</Button
		>
	</aside>
	{#if stage}<main bind:this={mainEl}>
			<div class="panel">
				<header>
					<div>
						<input
							class="stage-name"
							aria-label="Stage name"
							bind:value={stage.label}
							oninput={() => editBehaviourField(`${stagePath}.label`, 'Rename stage')}
							onblur={renameOwnedTag}
						/>
					</div>
				</header>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>Content</h3>
							<p>Choose which content and wallpaper are used during this stage.</p>
						</div>
					</div>
					<div class="toggle-row">
						<div>
							<strong>Active tags</strong>{#if previous}<small
									>Previous stage: {previous.content.tags
										? `${previous.content.tags.length} selected`
										: 'All content'}</small
								>{/if}
						</div>
						<Toggle
							ariaLabel="Active tags"
							checked={!!stage.content.tags}
							onchange={setRestriction}
						/>
					</div>
					{#if stage.content.tags}<TagPicker
							tags={stage.content.tags}
							id={`stage-content-${stage.id}`}
							path={`${stagePath}.content.tags`}
							onchange={(tags) => (stage.content.tags = tags)}
						/>
						{#if stage.content.owned_tag}<p class="owned-note">
								“{stage.content.owned_tag}” is this stage's own tag — the editor renames it with the
								stage, and the Popups tab uses it to put files in and out of this stage.
							</p>{/if}{/if}
					<MediaSlot
						slot={{ kind: 'stage_wallpaper', stage: stage.id }}
						mediaId={stage.content.wallpaper}
						title="Wallpaper"
						description="The wallpaper this stage sets. Every stage writes it outright, so leaving it empty is what keeps the one already in effect."
						emptyNote={inheritedWallpaper
							? `Keeps “${inheritedWallpaper}”.`
							: 'Keeps whatever wallpaper is already in effect.'}
						reveal={store.experienceTargetStageId === stage.id}
						onrevealed={() => (store.experienceTargetStageId = null)}
					/>
					<MediaSlot
						slot={{ kind: 'stage_audio', stage: stage.id }}
						mediaId={stage.content.audio}
						title="Background audio"
						description="The track this stage starts. Leaving it empty keeps the current track playing, without restarting it."
						emptyNote={inheritedAudio
							? 'Keeps the track selected by an earlier stage.'
							: 'Keeps the background rotation already playing.'}
					/>
				</section>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>On entry</h3>
							<p>Fire these effects once, after the transition into this stage finishes.</p>
						</div>
					</div>
					<div class="entry-effects">
						<div class="toggle-row">
							<div><strong>Show splash</strong><small>Uses the pack's splash.</small></div>
							<Toggle
								ariaLabel="Show splash on entry"
								checked={!!stage.on_enter?.splash}
								onchange={(on) => setEntryEffect('splash', on)}
							/>
						</div>
						<div class="toggle-row">
							<div><strong>Play sound</strong><small>Plays a popup-audio sting.</small></div>
							<Toggle
								ariaLabel="Play sound on entry"
								checked={!!stage.on_enter?.sound}
								onchange={(on) => setEntryEffect('sound', on)}
							/>
						</div>
						<div class="toggle-row">
							<div>
								<strong>Show notification</strong><small
									>Picks from the active notification pool.</small
								>
							</div>
							<Toggle
								ariaLabel="Show notification on entry"
								checked={!!stage.on_enter?.notification}
								onchange={(on) => setEntryEffect('notification', on)}
							/>
						</div>
						<label
							>Popup burst<input
								type="number"
								min="0"
								step="1"
								value={stage.on_enter?.popup_burst ?? 0}
								oninput={(event) => {
									const count = event.currentTarget.valueAsNumber;
									if (!Number.isFinite(count) || !stage) return;
									stage.on_enter ??= {};
									if (count > 0) stage.on_enter.popup_burst = Math.floor(count);
									else delete stage.on_enter.popup_burst;
									if (
										!stage.on_enter.splash &&
										!stage.on_enter.sound &&
										!stage.on_enter.notification &&
										!stage.on_enter.popup_burst
									)
										delete stage.on_enter;
									editBehaviourField(`${stagePath}.on_enter`, 'Change stage entry effects');
								}}
							/><small>Zero disables the burst. The user's popup limit still applies.</small></label
						>
					</div>
				</section>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>Events</h3>
							<p>Enable events and choose how frequently they spawn.</p>
						</div>
					</div>
					{#each eventDefs as def}<EventScheduleEditor
							label={def.label}
							value={eventValue(def.key)}
							previous={previous?.events[def.key]}
							defaultInterval={def.interval}
							onchange={(value) => setEvent(def.key, value)}
						/>{/each}
				</section>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>Moving windows</h3>
							<p>Control whether windows move around the screen.</p>
						</div>
						<Toggle
							ariaLabel="Enable window movement"
							checked={!!stage.movement}
							onchange={(on) => {
								if (on) stage.movement = { minimum_speed: 50, maximum_speed: 150 };
								else delete stage.movement;
								commitBehaviourEdit(`${stagePath}.movement`, 'Toggle window movement');
							}}
						/>
					</div>
					{#if stage.movement}<div class="fields">
							<label
								>Minimum speed<input
									type="number"
									bind:value={stage.movement.minimum_speed}
									oninput={() =>
										editBehaviourField(
											`${stagePath}.movement.minimum_speed`,
											'Edit movement speed'
										)}
								/><small>Previous: {previous?.movement?.minimum_speed ?? 'Off'}</small></label
							><label
								>Maximum speed<input
									type="number"
									bind:value={stage.movement.maximum_speed}
									oninput={() =>
										editBehaviourField(
											`${stagePath}.movement.maximum_speed`,
											'Edit movement speed'
										)}
								/><small>Previous: {previous?.movement?.maximum_speed ?? 'Off'}</small></label
							>
						</div>{/if}
				</section>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>Mitosis</h3>
							<p>Closing windows will have a chance of spawning more.</p>
						</div>
						<Toggle
							ariaLabel="Enable mitosis"
							checked={!!stage.mitosis}
							onchange={(on) => {
								if (on) stage.mitosis = { chance: 0.5, count: 2 };
								else delete stage.mitosis;
								commitBehaviourEdit(`${stagePath}.mitosis`, 'Toggle mitosis');
							}}
						/>
					</div>
					{#if stage.mitosis}<div class="fields">
							<label
								>Chance (0-1)<input
									type="number"
									min="0"
									max="1"
									step=".05"
									bind:value={stage.mitosis.chance}
									oninput={() => editBehaviourField(`${stagePath}.mitosis.chance`, 'Edit mitosis')}
								/><small>Previous: {previous?.mitosis?.chance ?? 'Off'}</small></label
							><label
								>Copies<input
									type="number"
									min="1"
									step="1"
									bind:value={stage.mitosis.count}
									oninput={() => editBehaviourField(`${stagePath}.mitosis.count`, 'Edit mitosis')}
								/><small>Previous: {previous?.mitosis?.count ?? 'Off'}</small></label
							>
						</div>{/if}
				</section>

				<section class="card">
					<div class="section-title">
						<div>
							<h3>Stage duration</h3>
							<p>
								{activeIndex === stages.length - 1
									? 'The final stage continues until the session ends.'
									: 'Choose how long these settings stay active.'}
							</p>
						</div>
					</div>
					{#if stage.end}<div class="fields">
							<label
								>Keep these settings for (minutes)<input
									type="number"
									min="0"
									value={(stage.end.duration_seconds ?? 300) / 60}
									oninput={(e) => {
										const n = e.currentTarget.valueAsNumber;
										if (Number.isFinite(n)) {
											stage.end!.duration_seconds = n * 60;
											editBehaviourField(
												`${stagePath}.end.duration_seconds`,
												'Edit stage duration'
											);
										}
									}}
								/>{#if stage.end.duration_seconds === 0}<small
										>The transition begins as soon as this stage is reached.</small
									>{/if}</label
							><Select
								label="Additional condition"
								value={stage.end.event_count ? stage.end.event_count.event : 'none'}
								options={[
									{ value: 'none', label: 'No event condition' },
									...eventDefs.map((d) => ({ value: d.key, label: `${d.label} spawned` }))
								]}
								onchange={(v) => {
									if (v === 'none') delete stage.end!.event_count;
									else stage.end!.event_count = { event: v as any, count: 10, scope: 'stage' };
									commitBehaviourEdit(`${stagePath}.end`, 'Change stage end condition');
								}}
							/>{#if stage.end.event_count}<label
									>Event count<input
										type="number"
										min="1"
										bind:value={stage.end.event_count.count}
										oninput={() =>
											editBehaviourField(
												`${stagePath}.end.event_count.count`,
												'Edit stage end condition'
											)}
									/></label
								><Select
									label="Advance when"
									value={stage.end.strategy}
									options={[
										{ value: 'any', label: 'Either condition is reached' },
										{ value: 'all', label: 'Both conditions are reached' }
									]}
									onchange={(v) => {
										stage.end!.strategy = v as 'any' | 'all';
										commitBehaviourEdit(`${stagePath}.end.strategy`, 'Change stage end condition');
									}}
								/>{/if}
						</div>
						{#if next && outgoing}<div class="next-summary">
								<span
									>Then change to <button onclick={() => (activeId = next.id)}>{next.label}</button
									></span
								><button class="transition-link" onclick={() => (activeId = outgoing.id)}
									>{transitionSummary()} <span aria-hidden="true">→</span></button
								>
							</div>{/if}{/if}
				</section>
			</div>
		</main>{:else if transition && transitionFrom && transitionTo}<TransitionEditor
			transitionId={transition.id}
			from={transitionFrom}
			to={transitionTo}
			onstage={(id) => (activeId = id)}
		/>{/if}
</section>
{#if removing}<Dialog
		title={`Remove “${removing.label}”?`}
		description={`This stage and its settings will be removed. Transitions to its neighbours will be reset.${
			removing.content.wallpaper ? ' Its wallpaper is cleared with it.' : ''
		}${removing.content.audio ? ' Its audio slot is cleared with it.' : ''}${removalNote()}`}
		buttons={[
			{ label: 'Cancel', onclick: () => (removing = null) },
			// Offered, never the default: an owned tag is almost always on media, and losing that
			// classification to a restructuring is a much worse trade than an unused tag left behind.
			...(removingClaims?.claimed
				? [
						{
							label: `Remove stage and “${removing.content.owned_tag}”`,
							destructive: true,
							onclick: () => confirmRemove(true)
						}
					]
				: []),
			{ label: 'Remove stage', destructive: true, primary: true, onclick: () => confirmRemove() }
		]}
		onclose={() => (removing = null)}
	/>{/if}

<style>
	.layout {
		display: flex;
		min-height: 0;
		flex: 1;
	}
	.layout > aside {
		display: flex;
		width: 192px;
		flex: none;
		padding: 12px;
		flex-direction: column;
		gap: 12px;
		border-right: 1px solid var(--ui-border);
		background: var(--ui-surface);
	}
	.tabs {
		min-height: 0;
		flex: 1;
		overflow-y: auto;
		scrollbar-gutter: stable;
		margin-right: -12px;
		padding-right: 8px;
	}
	main {
		flex: 1;
		min-width: 0;
		padding: 24px;
		overflow-y: auto;
	}
	main .panel {
		display: flex;
		width: 100%;
		max-width: 800px;
		min-width: 0;
		margin-inline: auto;
		flex-direction: column;
		gap: 14px;
	}
	.panel > header {
		display: flex;
		align-items: start;
		justify-content: space-between;
		gap: 16px;
	}
	.stage-name {
		width: calc(100% + 12px);
		margin-left: -6px;
		padding: 2px 6px;
		border: 1px solid transparent;
		border-radius: var(--ui-radius-sm);
		background: transparent;
		color: var(--ui-text);
		font-size: 17px;
		font-weight: 650;
		transition:
			border-color 120ms ease,
			background 120ms ease;
	}
	.stage-name:hover {
		border-color: var(--ui-border);
		background: var(--ui-bg);
	}
	.stage-name:focus {
		outline: none;
		border-color: var(--ui-focus);
		background: var(--ui-bg);
	}
	.section-title p {
		margin: 4px 0 0;
		color: var(--ui-muted);
		font-size: 12px;
	}
	.owned-note {
		margin: 0;
		color: var(--ui-muted);
		font-size: 11px;
		line-height: 1.45;
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
	.section-title,
	.toggle-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
	}
	.section-title h3 {
		margin: 0;
		font-size: 16px;
	}
	.toggle-row {
		padding-top: 10px;
		border-top: 1px solid var(--ui-border);
	}
	.toggle-row strong,
	.toggle-row small {
		display: block;
		font-size: 12px;
	}
	.toggle-row small {
		margin-top: 2px;
		color: var(--ui-muted);
		font-size: 10px;
	}
	.fields {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		align-items: start;
		gap: 12px;
	}
	.fields label {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 5px;
		color: var(--ui-text);
		font-size: 12px;
		font-weight: 600;
	}
	.entry-effects {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.entry-effects label {
		display: flex;
		flex-direction: column;
		gap: 5px;
		font-size: 12px;
		font-weight: 600;
	}
	.entry-effects input {
		width: 100%;
		height: 36px;
		padding: 0 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
	}
	.entry-effects label small {
		color: var(--ui-muted);
		font-size: 10px;
		font-weight: 400;
	}
	.fields input {
		width: 100%;
		height: 36px;
		padding: 0 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
		font: inherit;
		font-weight: 400;
		font-size: 13px;
	}
	.fields small {
		color: var(--ui-muted);
		font-size: 10px;
		font-weight: 400;
	}
	.next-summary {
		display: flex;
		padding-top: 12px;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		border-top: 1px solid var(--ui-border);
		color: var(--ui-muted);
		font-size: 12px;
	}
	.next-summary button {
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
	.next-summary .transition-link {
		padding: 7px 9px;
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		text-decoration: none;
	}
	.transition-link:hover {
		background: var(--ui-surface-raised);
	}
	@media (max-width: 700px) {
		.layout {
			flex-direction: column;
		}
		.layout > aside {
			width: 100%;
			padding-block: 0;
			align-items: center;
			flex-direction: row;
			border-right: 0;
			border-bottom: 1px solid var(--ui-border);
		}
		.tabs {
			overflow-x: auto;
			margin-right: 0;
			padding-right: 0;
		}
		main {
			padding: 16px;
		}
		.fields {
			grid-template-columns: 1fr;
		}
		.next-summary {
			align-items: flex-start;
			flex-direction: column;
		}
	}
</style>
