<script lang="ts">
	// One timeline stage: everything it shows, everything it fires, and how long it lasts.
	//
	// Split out of `TimelineEditor`, which is now only the timeline *as a list* -- the tab strip,
	// and the four edits that renumber it (add, duplicate, move, remove). Those address the timeline
	// whole; everything here addresses one stage's own fields, and the two had no reason to share a
	// file beyond having grown in one.
	//
	// The stage is looked up from the store by id rather than passed in, the same way
	// `TransitionEditor` does it: this component mutates it, and mutating an unbound prop trips
	// Svelte's ownership warning.
	import { clampScroll } from '$ui/scroll';
	import NumberField from '$ui/NumberField.svelte';
	import Select from '$ui/Select.svelte';
	import Toggle from '$ui/Toggle.svelte';
	import EventScheduleEditor from './EventScheduleEditor.svelte';
	import MediaSlot from './MediaSlot.svelte';
	import TagPicker from './TagPicker.svelte';
	import { api } from './api.js';
	import { fields } from './mutate.svelte.js';
	import { keys, query } from './query.svelte.js';
	import { stageTagName, takenTagNames } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { EventSchedule, Stage, TagAction } from './types.js';

	type Props = {
		stageId: string;
		/** Jumps to another stage or to a transition -- the "then change to…" line at the bottom. */
		onselect: (id: string) => void;
	};

	let { stageId, onselect }: Props = $props();

	const timelineQuery = query(keys.timeline, api.getTimeline);
	const slots = query(keys.mediaSlots, api.getMediaSlots);
	const tagRows = query(keys.tags, api.getTagRows);
	const stages = $derived(timelineQuery.current?.stages ?? []);
	const index = $derived(stages.findIndex((item) => item.id === stageId));
	const stage = $derived(stages[index]);
	const previous = $derived(index > 0 ? stages[index - 1] : undefined);
	const next = $derived(stages[index + 1]);
	const outgoing = $derived(
		next && stage
			? timelineQuery.current?.transitions.find(
					(item) => item.from_stage === stage.id && item.to_stage === next.id
				)
			: undefined
	);
	const isLast = $derived(index === stages.length - 1);

	const invalidates = [keys.timeline, keys.summary, keys.tags];

	/**
	 * Sends this stage, having applied `change` to the copy this view is showing.
	 *
	 * A stage is a few dozen small fields, so it goes whole rather than as one command per field —
	 * still one row and its children, and still a changeset the size of one stage. `retiring` and
	 * `tagActions` ride along for the edits that are also about media or tags, so a rename and the
	 * tag rename it causes cannot come apart under undo.
	 *
	 * Changes accumulate into one draft per stage. A stage is sent whole, so building each command
	 * from the last *fetched* copy would mean toggling mitosis mid-rename sent the old name back.
	 */
	function write(
		change: (draft: Stage) => void,
		label: string,
		options: { debounce?: boolean; retiring?: number[]; tagActions?: TagAction[] } = {}
	) {
		if (!stage) return;
		const id = stage.id;
		fields.edit<Stage>({
			entity: `stage:${id}`,
			base: () => structuredClone($state.snapshot(stage)) as Stage,
			change,
			label,
			invalidates,
			send: (draft) =>
				api.updateStages(
					[{ id, stage: draft }],
					options.retiring ?? [],
					options.tagActions ?? [],
					label
				),
			debounce: options.debounce
		});
	}

	/** The stage as the author has it: their unsent edits if any, else what was fetched. */
	const shown = $derived((stage && fields.draftFor<Stage>(`stage:${stage.id}`)) ?? stage);

	let mainEl = $state<HTMLElement>();
	let audioPickerStage = $state<string | null>(null);

	// WebKitGTK doesn't reliably clamp scrollTop when the panel's content shrinks,
	// leaving a shorter stage blank and unscrollable.
	$effect(() => {
		stageId;
		mainEl?.scrollTo(0, 0);
	});

	const eventDefs = [
		{ key: 'popup', label: 'Popups', interval: 30 },
		{ key: 'web', label: 'Web links', interval: 300 },
		{ key: 'notification', label: 'Notifications', interval: 300 },
		{ key: 'prompt', label: 'Prompts', interval: 90 },
		{ key: 'sound', label: 'Sounds', interval: 90 }
	] as const;

	// What this stage's wallpaper would be if it sets none: the nearest earlier stage that sets
	// one, or the pack's own. A stage's wallpaper is an absolute write, so "empty" isn't "none" --
	// it's "whatever is already up", and the author can only judge that if we name it.
	const inheritedWallpaperId = $derived.by(() => {
		for (let earlier = index - 1; earlier >= 0; earlier--) {
			const id = stages[earlier].content.wallpaper;
			if (id != null) return id;
		}
		return slots.current?.wallpaper ?? undefined;
	});
	// The behaviour stores a media id; the author knows the file by its name, so resolve it against
	// the file grid. A file that has since left the pack resolves to nothing, and we fall back to
	// the generic wording rather than naming an id.
	const inheritedWallpaper = $derived(
		inheritedWallpaperId == null ? undefined : store.fileById(inheritedWallpaperId)?.file_name
	);

	// ── The stage's own tag ────────────────────────────────────────────────────
	//
	// A stage that restricts its content needs a tag for the "Appears in" strip to write, and
	// leaving authors to invent one per stage gives the pack arbitrary names that mean nothing to
	// anyone reading it. So the editor creates one, keeps its name in step with the stage's, and
	// takes it away again when the stage goes and nothing else is holding it. Naming and the
	// claim check live in `stageTags.ts`; the removal moment is in `TimelineEditor`, which owns the
	// confirmation. These are the other two.

	/** Every tag name the pack already has, so a new one cannot land on an existing classification. */
	function taken(except?: string) {
		const names = takenTagNames(tagRows.current?.map((row) => row.name) ?? store.allTags);
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
			write((draft) => {
				delete draft.content.tags;
				delete draft.content.owned_tag;
			}, 'Change stage content');
			return;
		}
		const tag = stageTagName(stage.label, taken());
		// `media: null` is "every file in the pack", resolved server-side — the seeding rule, which
		// keeps the stage showing what it showed a moment before its tags were switched on.
		write(
			(draft) => {
				draft.content.tags = [tag];
				draft.content.owned_tag = tag;
			},
			'Restrict stage content',
			{ tagActions: [{ kind: 'apply', tag, media: null }] }
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
		// The label the author has actually typed, not the one last fetched. Blur lands inside the
		// name field's debounce window, so the stored label is still the old one — reading that made
		// this decide the tag was already correctly named and leave it behind.
		const label = shown?.label ?? stage.label;
		const renamed = stageTagName(label, taken(owned));
		if (renamed === owned) return;
		// Written into the same draft as the name, so the rename and the tag rename it causes are
		// one command and one undo entry.
		write(
			(draft) => {
				draft.content.tags = (draft.content.tags ?? []).map((tag) =>
					tag === owned ? renamed : tag
				);
				draft.content.owned_tag = renamed;
			},
			'Rename stage',
			{ tagActions: [{ kind: 'rename', from: owned, to: renamed }] }
		);
	}

	function setEvent(key: keyof Stage['events'], value?: EventSchedule) {
		write((draft) => {
			if (value) draft.events[key] = value;
			else delete draft.events[key];
		}, 'Change stage events');
	}

	function setAudioMode(mode: string) {
		if (!stage) return;
		// The track this stage was naming is deliberately let go of: leaving it behind would litter
		// the pack with a sound marked out of popups and referenced by nothing.
		const retiring = stage.content.audio != null ? [stage.content.audio] : [];
		audioPickerStage = mode === 'specific' ? stage.id : null;
		write(
			(draft) => {
				delete draft.content.audio;
				if (mode === 'random') draft.content.audio_random = true;
				else delete draft.content.audio_random;
			},
			'Change stage audio',
			{ retiring }
		);
	}

	/** The prompt block a draft edit is about to write into, created on first use. */
	function promptOf(draft: Stage) {
		draft.prompt ??= { timeouts_enabled: true, timeout_multiplier: 1 };
		return draft.prompt;
	}

	function setPromptPopups(enabled: boolean) {
		write((draft) => {
			const prompt = promptOf(draft);
			if (enabled) prompt.popup_burst ??= 5;
			else delete prompt.popup_burst;
		}, 'Toggle prompt popups');
	}

	function setEntryPopups(enabled: boolean) {
		write((draft) => {
			draft.on_enter ??= {};
			if (enabled) draft.on_enter.popup_burst ??= 5;
			else delete draft.on_enter.popup_burst;
		}, 'Toggle stage entry popups');
	}

	function transitionSummary() {
		if (!outgoing || outgoing.duration_seconds === 0) return 'Immediately';
		const minutes = outgoing.duration_seconds / 60;
		return `Gradually over ${Number.isInteger(minutes) ? `${minutes} minute${minutes === 1 ? '' : 's'}` : `${outgoing.duration_seconds} seconds`}`;
	}
</script>

<main bind:this={mainEl} use:clampScroll>
	<div class="panel">
		<header>
			<div>
				<input
					class="stage-name"
					aria-label="Stage name"
					value={shown?.label ?? ''}
					oninput={(event) =>
						write((draft) => (draft.label = event.currentTarget.value), 'Rename stage', {
							debounce: true
						})}
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
				<Toggle ariaLabel="Active tags" checked={!!shown.content.tags} onchange={setRestriction} />
			</div>
			{#if shown.content.tags}<TagPicker
					tags={shown.content.tags}
					id={`stage-content-${stage.id}`}
					onchange={(tags, label) => write((draft) => (draft.content.tags = tags), label)}
				/>
				{#if shown.content.owned_tag}<p class="owned-note">
						“{shown.content.owned_tag}” is this stage's own tag — the editor renames it with the
						stage, and the Popups tab uses it to put files in and out of this stage.
					</p>{/if}{/if}
			<MediaSlot
				slot={{ kind: 'stage_wallpaper', stage: stage.id }}
				mediaId={shown.content.wallpaper}
				title="Wallpaper"
				description="The wallpaper this stage sets. Every stage writes it outright, so leaving it empty is what keeps the one already in effect."
				emptyNote={inheritedWallpaper
					? `Keeps “${inheritedWallpaper}”.`
					: 'Keeps whatever wallpaper is already in effect.'}
				reveal={store.experienceTargetStageId === stage.id}
				onrevealed={() => (store.experienceTargetStageId = null)}
			/>
			<div class="audio-choice">
				<Select
					label="Background audio"
					value={shown.content.audio != null || audioPickerStage === stage.id
						? 'specific'
						: shown.content.audio_random
							? 'random'
							: 'keep'}
					options={[
						{ value: 'keep', label: 'Continue current audio' },
						{ value: 'random', label: 'Switch to a random track using active tags' },
						{ value: 'specific', label: 'Switch to a specific track' }
					]}
					onchange={setAudioMode}
				/>
				{#if shown.content.audio != null || audioPickerStage === stage.id}
					<MediaSlot
						slot={{ kind: 'stage_audio', stage: stage.id }}
						mediaId={shown.content.audio}
						title="Specific track"
						description="Choose the exact track this stage starts."
						emptyNote="Choose a track."
						showHeader={false}
					/>
				{/if}
			</div>
		</section>

		<section class="card">
			<div class="section-title">
				<div>
					<h3>On entry</h3>
					<p>Fire these effects once, after the transition into this stage finishes.</p>
				</div>
			</div>
			<div class="entry-effects">
				<MediaSlot
					slot={{ kind: 'stage_entry_splash', stage: stage.id }}
					mediaId={shown.on_enter?.splash}
					title="Splash"
					description="A specific image or video shown on entry."
					emptyNote="No splash is shown on entry."
				/>
				<MediaSlot
					slot={{ kind: 'stage_entry_sound', stage: stage.id }}
					mediaId={shown.on_enter?.sound}
					title="Sound"
					description="A specific sound played on entry."
					emptyNote="No sound is played on entry."
				/>
				<label
					>Notification<textarea
						rows="2"
						value={shown.on_enter?.notification ?? ''}
						placeholder="No notification"
						oninput={(event) => {
							const value = event.currentTarget.value;
							write(
								(draft) => {
									draft.on_enter ??= {};
									if (value) draft.on_enter.notification = value;
									else delete draft.on_enter.notification;
								},
								'Edit stage notification',
								{ debounce: true }
							);
						}}></textarea><small>Custom text sent as a desktop notification.</small></label
				>
				<div class="optional-effect">
					<div class="toggle-row">
						<div>
							<strong>Spawn popups</strong><small>Runs once when this stage begins.</small>
						</div>
						<Toggle
							ariaLabel="Spawn popups on stage entry"
							checked={shown.on_enter?.popup_burst != null}
							onchange={setEntryPopups}
						/>
					</div>
					{#if shown.on_enter?.popup_burst != null}<NumberField
							label="Number of popups"
							description="The user's popup limit still applies."
							min={1}
							step={1}
							value={shown.on_enter.popup_burst}
							oninput={(count) => {
								// An empty field is not a burst of zero -- and not a burst of one either.
								// Leave the stored count alone until a real number is typed.
								if (count === null) return;
								write(
									(draft) => {
										draft.on_enter ??= {};
										draft.on_enter.popup_burst = Math.max(1, Math.floor(count));
									},
									'Edit stage entry popups',
									{ debounce: true }
								);
							}}
						/>{/if}
				</div>
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
					value={shown.events[def.key]}
					previous={previous?.events[def.key]}
					defaultInterval={def.interval}
					onchange={(value) => setEvent(def.key, value)}
				>
					{#if def.key === 'prompt'}<div class="prompt-settings">
							<div class="toggle-row">
								<div>
									<strong>Enforce prompt deadlines</strong><small
										>Turn off to let prompts wait indefinitely.</small
									>
								</div>
								<Toggle
									ariaLabel="Enforce prompt deadlines"
									checked={shown.prompt?.timeouts_enabled !== false}
									onchange={(on) =>
										write(
											(draft) => (promptOf(draft).timeouts_enabled = on),
											'Change prompt deadlines'
										)}
								/>
							</div>
							{#if shown.prompt?.timeouts_enabled !== false}<NumberField
									label="Time allowance"
									description="Multiplies every prompt's explicit or automatic time limit."
									min={0.1}
									step={0.1}
									suffix="×"
									value={shown.prompt?.timeout_multiplier ?? 1}
									oninput={(multiplier) => {
										if (multiplier === null) return;
										write(
											(draft) => (promptOf(draft).timeout_multiplier = Math.max(0.1, multiplier)),
											'Edit prompt time allowance',
											{ debounce: true }
										);
									}}
								/>{/if}
							<div class="optional-effect">
								<div class="toggle-row">
									<div>
										<strong>Spawn popups</strong><small
											>Applied to wrong answers and expired prompts.</small
										>
									</div>
									<Toggle
										ariaLabel="Spawn popups for an incorrect prompt"
										checked={shown.prompt?.popup_burst != null}
										onchange={setPromptPopups}
									/>
								</div>
								{#if shown.prompt?.popup_burst != null}<NumberField
										label="Number of popups"
										description="The user's popup limit still applies."
										min={1}
										step={1}
										value={shown.prompt.popup_burst}
										oninput={(count) => {
											if (count === null) return;
											write(
												(draft) => (promptOf(draft).popup_burst = Math.max(1, Math.floor(count))),
												'Edit prompt popups',
												{ debounce: true }
											);
										}}
									/>{/if}
							</div>
							<MediaSlot
								slot={{ kind: 'stage_prompt_sound', stage: stage.id }}
								mediaId={shown.prompt?.sound}
								title="Sound"
								description="A specific sound played for a wrong answer or timeout."
								emptyNote="No sound consequence."
							/>
						</div>{/if}
				</EventScheduleEditor>{/each}
		</section>

		<section class="card">
			<div class="section-title">
				<div>
					<h3>Moving windows</h3>
					<p>Control whether windows move around the screen.</p>
				</div>
				<Toggle
					ariaLabel="Enable window movement"
					checked={!!shown.movement}
					onchange={(on) =>
						write((draft) => {
							if (on) draft.movement = { minimum_speed: 50, maximum_speed: 150 };
							else delete draft.movement;
						}, 'Toggle window movement')}
				/>
			</div>
			{#if shown.movement}<div class="fields">
					<NumberField
						label="Minimum speed"
						description={`Previous: ${previous?.movement?.minimum_speed ?? 'Off'}`}
						value={shown.movement.minimum_speed}
						oninput={(speed) => {
							if (speed === null) return;
							write(
								(draft) => draft.movement && (draft.movement.minimum_speed = speed),
								'Edit movement speed',
								{ debounce: true }
							);
						}}
					/>
					<NumberField
						label="Maximum speed"
						description={`Previous: ${previous?.movement?.maximum_speed ?? 'Off'}`}
						value={shown.movement.maximum_speed}
						oninput={(speed) => {
							if (speed === null) return;
							write(
								(draft) => draft.movement && (draft.movement.maximum_speed = speed),
								'Edit movement speed',
								{ debounce: true }
							);
						}}
					/>
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
					checked={!!shown.mitosis}
					onchange={(on) =>
						write((draft) => {
							if (on) draft.mitosis = { chance: 0.5, count: 2 };
							else delete draft.mitosis;
						}, 'Toggle mitosis')}
				/>
			</div>
			{#if shown.mitosis}<div class="fields">
					<NumberField
						label="Chance (0-1)"
						description={`Previous: ${previous?.mitosis?.chance ?? 'Off'}`}
						min={0}
						max={1}
						step={0.05}
						value={shown.mitosis.chance}
						oninput={(chance) => {
							if (chance === null) return;
							write((draft) => draft.mitosis && (draft.mitosis.chance = chance), 'Edit mitosis', {
								debounce: true
							});
						}}
					/>
					<NumberField
						label="Copies"
						description={`Previous: ${previous?.mitosis?.count ?? 'Off'}`}
						min={1}
						step={1}
						value={shown.mitosis.count}
						oninput={(count) => {
							if (count === null) return;
							write((draft) => draft.mitosis && (draft.mitosis.count = count), 'Edit mitosis', {
								debounce: true
							});
						}}
					/>
				</div>{/if}
		</section>

		<section class="card">
			<div class="section-title">
				<div>
					<h3>Stage duration</h3>
					<p>
						{isLast
							? 'The final stage continues until the session ends.'
							: 'Choose how long these settings stay active.'}
					</p>
				</div>
			</div>
			{#if shown.end}<div class="fields">
					<NumberField
						label="Keep these settings for (minutes)"
						description={shown.end.duration_seconds === 0
							? 'The transition begins as soon as this stage is reached.'
							: undefined}
						min={0}
						value={(shown.end.duration_seconds ?? 300) / 60}
						oninput={(minutes) => {
							if (minutes === null) return;
							write(
								(draft) => draft.end && (draft.end.duration_seconds = minutes * 60),
								'Edit stage duration',
								{ debounce: true }
							);
						}}
					/><Select
						label="Additional condition"
						value={shown.end.event_count ? shown.end.event_count.event : 'none'}
						options={[
							{ value: 'none', label: 'No event condition' },
							...eventDefs.map((def) => ({ value: def.key, label: `${def.label} spawned` }))
						]}
						onchange={(value) =>
							write((draft) => {
								if (!draft.end) return;
								if (value === 'none') delete draft.end.event_count;
								else
									draft.end.event_count = {
										event: value as (typeof eventDefs)[number]['key'],
										count: 10,
										scope: 'stage'
									};
							}, 'Change stage end condition')}
					/>{#if shown.end.event_count}<NumberField
							label="Event count"
							min={1}
							value={shown.end.event_count.count}
							oninput={(count) => {
								if (count === null) return;
								write(
									(draft) => draft.end?.event_count && (draft.end.event_count.count = count),
									'Edit stage end condition',
									{ debounce: true }
								);
							}}
						/><Select
							label="Advance when"
							value={shown.end.strategy}
							options={[
								{ value: 'any', label: 'Either condition is reached' },
								{ value: 'all', label: 'Both conditions are reached' }
							]}
							onchange={(value) =>
								write(
									(draft) => draft.end && (draft.end.strategy = value as 'any' | 'all'),
									'Change stage end condition'
								)}
						/>{/if}
				</div>
				{#if next && outgoing}<div class="next-summary">
						<span
							>Then change to <button onclick={() => onselect(next.id)}>{next.label}</button></span
						><button class="transition-link" onclick={() => onselect(outgoing.id)}
							>{transitionSummary()} <span aria-hidden="true">→</span></button
						>
					</div>{/if}{/if}
		</section>
	</div>
</main>

<style>
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
	.audio-choice,
	.prompt-settings,
	.optional-effect {
		display: flex;
		min-width: 0;
		flex-direction: column;
		gap: 12px;
	}
	.prompt-settings {
		margin-top: 14px;
		padding: 0 0 2px 16px;
	}
	.prompt-settings > .toggle-row:first-child {
		padding-top: 0;
		border-top: 0;
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
	.entry-effects textarea {
		width: 100%;
		min-height: 68px;
		padding: 8px 9px;
		border: 1px solid var(--ui-border);
		border-radius: var(--ui-radius-sm);
		background: var(--ui-bg);
		color: var(--ui-text);
		font: inherit;
		font-weight: 400;
		resize: vertical;
	}
	.entry-effects label small {
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
