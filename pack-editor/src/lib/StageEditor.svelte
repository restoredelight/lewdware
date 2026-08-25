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
	import { mutate } from './mutate.svelte.js';
	import DebouncedField from './DebouncedField.svelte';
	import { keys, query } from './query.svelte.js';
	import { stageTagName, takenTagNames } from './stageTags.js';
	import { store } from './store.svelte.js';
	import type { EventKind, EventSchedule, Stage, TagAction } from './types.js';

	type Props = {
		stageId: string;
		/** Jumps to another stage or to a transition -- the "then change to…" line at the bottom. */
		onselect: (id: string) => void;
	};

	let { stageId, onselect }: Props = $props();

	const timelineQuery = query(keys.timeline, api.timeline.get);
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

	/** Sends one field of this stage. Nothing else about the stage travels with the change. */
	function write(run: () => Promise<unknown>, label: string) {
		void mutate(run as () => Promise<void>, { label, invalidates });
	}

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
		const id = stage.id;
		if (!on) {
			write(
				() => api.stage.setContentTags(id, null, null, [], 'Change stage content'),
				'Change stage content'
			);
			return;
		}
		const tag = stageTagName(stage.label, taken());
		// `media: null` is "every file in the pack", resolved server-side — the seeding rule, which
		// keeps the stage showing what it showed a moment before its tags were switched on.
		write(
			() =>
				api.stage.setContentTags(
					id,
					[tag],
					tag,
					[{ kind: 'apply', tag, media: null }],
					'Restrict stage content'
				),
			'Restrict stage content'
		);
	}

	/**
	 * The tag rename that goes with renaming this stage to `label`, or nothing if none is due.
	 *
	 * Returned rather than sent, so it can travel *with* the rename. `set_stage_label` takes the tag
	 * actions for exactly this reason: the stage's name and the name of the tag the editor made for
	 * it are one thing the author did, and two commands would be two undo entries — one of which
	 * could fail on its own and leave the two disagreeing.
	 *
	 * Only a tag no other stage selects by. A rename is lossless everywhere else it appears (the
	 * lists are joins, so media, pools and groups follow the id), but another stage reading this
	 * name is another author decision, and this one is bookkeeping.
	 *
	 * Nothing rewrites `content.tags`: the selection holds tag *ids*, so renaming the row is the
	 * whole of it.
	 */
	function ownedTagRename(label: string): TagAction[] {
		const owned = stage?.content.owned_tag;
		if (!stage || !owned) return [];
		const id = stage.id;
		if (stages.some((other) => other.id !== id && (other.content.tags ?? []).includes(owned)))
			return [];
		const renamed = stageTagName(label, taken(owned));
		if (renamed === owned) return [];
		return [{ kind: 'rename', from: owned, to: renamed }];
	}

	function setEvent(key: EventKind, value?: EventSchedule) {
		if (!stage) return;
		write(
			() => api.stage.setEvent(stage.id, key, value ?? null, 'Change stage events'),
			'Change stage events'
		);
	}

	function setAudioMode(mode: string) {
		if (!stage) return;
		const id = stage.id;
		// The track this stage was naming is deliberately let go of: leaving it behind would litter
		// the pack with a sound marked out of popups and referenced by nothing.
		const retiring = stage.content.audio != null ? [stage.content.audio] : [];
		audioPickerStage = mode === 'specific' ? id : null;
		write(
			() => api.stage.setAudioRandom(id, mode === 'random', retiring, 'Change stage audio'),
			'Change stage audio'
		);
	}

	function setPromptPopups(enabled: boolean) {
		if (!stage) return;
		write(
			() =>
				api.stage.setPromptPopupBurst(
					stage.id,
					enabled ? (stage.prompt?.popup_burst ?? 5) : null,
					'Toggle prompt popups'
				),
			'Toggle prompt popups'
		);
	}

	function setEntryPopups(enabled: boolean) {
		if (!stage) return;
		write(
			() =>
				api.stage.setEntryPopupBurst(
					stage.id,
					enabled ? (stage.on_enter?.popup_burst ?? 5) : null,
					'Toggle stage entry popups'
				),
			'Toggle stage entry popups'
		);
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
				<DebouncedField
					value={stage?.label ?? ''}
					label="Rename stage"
					{invalidates}
					oncommit={(value: string, label: string) =>
						// The tag the stage owns is renamed in the same command, so one undo takes back
						// the whole rename rather than half of it.
						api.stage.setLabel(stage!.id, value, ownedTagRename(value), label)}
				>
					{#snippet field(draft, set, commit)}
						<input
							class="stage-name"
							aria-label="Stage name"
							value={draft}
							oninput={(event) => set(event.currentTarget.value)}
							onblur={commit}
						/>
					{/snippet}
				</DebouncedField>
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
				<Toggle ariaLabel="Active tags" checked={!!stage.content.tags} onchange={setRestriction} />
			</div>
			{#if stage.content.tags}<TagPicker
					tags={stage.content.tags}
					id={`stage-content-${stage.id}`}
					onchange={(tag, added, label) =>
						write(
							() =>
								added
									? api.stage.addTag(stage.id, tag, label)
									: api.stage.removeTag(stage.id, tag, label),
							label
						)}
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
			<div class="audio-choice">
				<Select
					label="Background audio"
					value={stage.content.audio != null || audioPickerStage === stage.id
						? 'specific'
						: stage.content.audio_random
							? 'random'
							: 'keep'}
					options={[
						{ value: 'keep', label: 'Continue current audio' },
						{ value: 'random', label: 'Switch to a random track using active tags' },
						{ value: 'specific', label: 'Switch to a specific track' }
					]}
					onchange={setAudioMode}
				/>
				{#if stage.content.audio != null || audioPickerStage === stage.id}
					<MediaSlot
						slot={{ kind: 'stage_audio', stage: stage.id }}
						mediaId={stage.content.audio}
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
					mediaId={stage.on_enter?.splash}
					title="Splash"
					description="A specific image or video shown on entry."
					emptyNote="No splash is shown on entry."
				/>
				<MediaSlot
					slot={{ kind: 'stage_entry_sound', stage: stage.id }}
					mediaId={stage.on_enter?.sound}
					title="Sound"
					description="A specific sound played on entry."
					emptyNote="No sound is played on entry."
				/>
				<label
					>Notification<DebouncedField
						value={stage.on_enter?.notification ?? ''}
						label="Edit stage notification"
						{invalidates}
						oncommit={(value: string, label: string) =>
							api.stage.setEntryNotification(stage.id, value ? value : null, label)}
					>
						{#snippet field(draft, set, commit)}
							<textarea
								rows="2"
								value={draft}
								placeholder="No notification"
								oninput={(event) => set(event.currentTarget.value)}
								onblur={commit}></textarea>
						{/snippet}
					</DebouncedField><small>Custom text sent as a desktop notification.</small></label
				>
				<div class="optional-effect">
					<div class="toggle-row">
						<div>
							<strong>Spawn popups</strong><small>Runs once when this stage begins.</small>
						</div>
						<Toggle
							ariaLabel="Spawn popups on stage entry"
							checked={stage.on_enter?.popup_burst != null}
							onchange={setEntryPopups}
						/>
					</div>
					{#if stage.on_enter?.popup_burst != null}<DebouncedField
							value={stage.on_enter.popup_burst}
							label="Edit stage entry popups"
							{invalidates}
							oncommit={(value: number | null | undefined, label: string) =>
								api.stage.setEntryPopupBurst(stage.id, value ?? null, label)}
						>
							{#snippet field(draft, set, commit)}
								<NumberField
									label="Number of popups"
									description="The user's popup limit still applies."
									min={1}
									step={1}
									value={draft}
									oninput={(count) => count !== null && set(Math.max(1, Math.floor(count)))}
									onchange={() => commit()}
								/>
							{/snippet}
						</DebouncedField>{/if}
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
					value={stage.events[def.key]}
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
									checked={stage.prompt?.timeouts_enabled !== false}
									onchange={(on) =>
										write(
											() =>
												api.stage.setPromptTimeoutsEnabled(stage.id, on, 'Change prompt deadlines'),
											'Change prompt deadlines'
										)}
								/>
							</div>
							{#if stage.prompt?.timeouts_enabled !== false}<DebouncedField
									value={stage.prompt?.timeout_multiplier ?? 1}
									label="Edit prompt time allowance"
									{invalidates}
									oncommit={(value: number, label: string) =>
										api.stage.setPromptTimeoutMultiplier(stage.id, value, label)}
								>
									{#snippet field(draft, set, commit)}
										<NumberField
											label="Time allowance"
											description="Multiplies every prompt's explicit or automatic time limit."
											min={0.1}
											step={0.1}
											suffix="×"
											value={draft}
											oninput={(multiplier) =>
												multiplier !== null && set(Math.max(0.1, multiplier))}
											onchange={() => commit()}
										/>
									{/snippet}
								</DebouncedField>{/if}
							<div class="optional-effect">
								<div class="toggle-row">
									<div>
										<strong>Spawn popups</strong><small
											>Applied to wrong answers and expired prompts.</small
										>
									</div>
									<Toggle
										ariaLabel="Spawn popups for an incorrect prompt"
										checked={stage.prompt?.popup_burst != null}
										onchange={setPromptPopups}
									/>
								</div>
								{#if stage.prompt?.popup_burst != null}<DebouncedField
										value={stage.prompt.popup_burst}
										label="Edit prompt popups"
										{invalidates}
										oncommit={(value: number | null | undefined, label: string) =>
											api.stage.setPromptPopupBurst(stage.id, value ?? null, label)}
									>
										{#snippet field(draft, set, commit)}
											<NumberField
												label="Number of popups"
												description="The user's popup limit still applies."
												min={1}
												step={1}
												value={draft}
												oninput={(count) => count !== null && set(Math.max(1, Math.floor(count)))}
												onchange={() => commit()}
											/>
										{/snippet}
									</DebouncedField>{/if}
							</div>
							<MediaSlot
								slot={{ kind: 'stage_prompt_sound', stage: stage.id }}
								mediaId={stage.prompt?.sound}
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
					checked={!!stage.movement}
					onchange={(on) =>
						write(
							() =>
								api.stage.setMovement(
									stage.id,
									on ? { minimum_speed: 50, maximum_speed: 150 } : null,
									'Toggle window movement'
								),
							'Toggle window movement'
						)}
				/>
			</div>
			{#if stage.movement}<div class="fields">
					<DebouncedField
						value={stage.movement.minimum_speed}
						label="Edit movement speed"
						{invalidates}
						oncommit={(value: number | null | undefined, label: string) =>
							api.stage.setMovementSpeed(stage.id, value ?? null, null, label)}
					>
						{#snippet field(draft, set, commit)}
							<NumberField
								label="Minimum speed"
								description={`Previous: ${previous?.movement?.minimum_speed ?? 'Off'}`}
								value={draft}
								oninput={(speed) => speed !== null && set(speed)}
								onchange={() => commit()}
							/>
						{/snippet}
					</DebouncedField>
					<DebouncedField
						value={stage.movement.maximum_speed}
						label="Edit movement speed"
						{invalidates}
						oncommit={(value: number | null | undefined, label: string) =>
							api.stage.setMovementSpeed(stage.id, null, value ?? null, label)}
					>
						{#snippet field(draft, set, commit)}
							<NumberField
								label="Maximum speed"
								description={`Previous: ${previous?.movement?.maximum_speed ?? 'Off'}`}
								value={draft}
								oninput={(speed) => speed !== null && set(speed)}
								onchange={() => commit()}
							/>
						{/snippet}
					</DebouncedField>
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
					onchange={(on) =>
						write(
							() =>
								api.stage.setMitosis(
									stage.id,
									on ? { chance: 0.5, count: 2 } : null,
									'Toggle mitosis'
								),
							'Toggle mitosis'
						)}
				/>
			</div>
			{#if stage.mitosis}<div class="fields">
					<DebouncedField
						value={stage.mitosis.chance}
						label="Edit mitosis"
						{invalidates}
						oncommit={(value: number | null | undefined, label: string) =>
							api.stage.setMitosisValues(stage.id, value ?? null, null, label)}
					>
						{#snippet field(draft, set, commit)}
							<NumberField
								label="Chance (0-1)"
								description={`Previous: ${previous?.mitosis?.chance ?? 'Off'}`}
								min={0}
								max={1}
								step={0.05}
								value={draft}
								oninput={(chance) => chance !== null && set(chance)}
								onchange={() => commit()}
							/>
						{/snippet}
					</DebouncedField>
					<DebouncedField
						value={stage.mitosis.count}
						label="Edit mitosis"
						{invalidates}
						oncommit={(value: number | null | undefined, label: string) =>
							api.stage.setMitosisValues(stage.id, null, value ?? null, label)}
					>
						{#snippet field(draft, set, commit)}
							<NumberField
								label="Copies"
								description={`Previous: ${previous?.mitosis?.count ?? 'Off'}`}
								min={1}
								step={1}
								value={draft}
								oninput={(count) => count !== null && set(count)}
								onchange={() => commit()}
							/>
						{/snippet}
					</DebouncedField>
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
			{#if stage.end}<div class="fields">
					<DebouncedField
						value={(stage.end?.duration_seconds ?? 300) / 60}
						label="Edit stage duration"
						{invalidates}
						oncommit={(value: number | null, label: string) =>
							api.stage.setEndDuration(stage.id, value === null ? null : value * 60, label)}
					>
						{#snippet field(draft, set, commit)}
							<NumberField
								label="Keep these settings for (minutes)"
								description={stage.end?.duration_seconds === 0
									? 'The transition begins as soon as this stage is reached.'
									: undefined}
								min={0}
								value={draft}
								oninput={(minutes) => minutes !== null && set(minutes)}
								onchange={() => commit()}
							/>
						{/snippet}
					</DebouncedField><Select
						label="Additional condition"
						value={stage.end.event_count ? stage.end.event_count.event : 'none'}
						options={[
							{ value: 'none', label: 'No event condition' },
							...eventDefs.map((def) => ({ value: def.key, label: `${def.label} spawned` }))
						]}
						onchange={(value) =>
							write(
								() =>
									api.stage.setEndEventCount(
										stage.id,
										value === 'none'
											? null
											: {
													event: value as (typeof eventDefs)[number]['key'],
													count: 10,
													scope: 'stage'
												},
										'Change stage end condition'
									),
								'Change stage end condition'
							)}
					/>{#if stage.end.event_count}<DebouncedField
							value={stage.end?.event_count?.count ?? null}
							label="Edit stage end condition"
							{invalidates}
							oncommit={(value: number | null, label: string) =>
								api.stage.setEndEventCount(
									stage.id,
									stage.end?.event_count && value !== null
										? { ...stage.end.event_count, count: value }
										: (stage.end?.event_count ?? null),
									label
								)}
						>
							{#snippet field(draft, set, commit)}
								<NumberField
									label="Event count"
									min={1}
									value={draft}
									oninput={(count) => count !== null && set(count)}
									onchange={() => commit()}
								/>
							{/snippet}
						</DebouncedField><Select
							label="Advance when"
							value={stage.end.strategy}
							options={[
								{ value: 'any', label: 'Either condition is reached' },
								{ value: 'all', label: 'Both conditions are reached' }
							]}
							onchange={(value) =>
								write(
									() =>
										api.stage.setEndStrategy(
											stage.id,
											value as 'any' | 'all',
											'Change stage end condition'
										),
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
