<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte';
	import Toggle from '$ui/Toggle.svelte';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import Select from '$ui/Select.svelte';
	import Popover from '$ui/Popover.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import { Icon, QuestionMarkCircle } from '$icons';
	import type { CrowdingDto, Frequency, ModeId, QuietHoursDto, RuleDto, TimeOfDay } from './types';
	import { api } from './api';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';

	const DAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

	// Attached to the frequency input rather than set out as a panel: it explains one number, and
	// only matters to someone looking at that number. Deliberately does not restate the randomness
	// -- the timing section already says that, a few lines above.
	// Panic's scope, as the one thing a user actually wants to say: "don't come back for...".
	// Every scope option is a point on this axis, and the far end -- "until I turn it back on" --
	// is the enable toggle at the top of this page, so only finite durations belong here.
	const PANIC_COOLDOWNS = [
		{ value: '0', label: 'Don’t pause' },
		{ value: '30', label: '30 minutes' },
		{ value: '120', label: '2 hours' },
		{ value: '480', label: '8 hours' },
		{ value: '1440', label: '24 hours' }
	];

	const RATE_EXPLANATION =
		'Sometimes you will get fewer sessions than this - Lewdware only starts them while ' +
		'you’re actually at your computer. It learns which hours you’re usually around, so this ' +
		'gets less common over time.';

	// Kept fresh by the `supervisor:status` push event; see +page.svelte.
	const status = $derived(store.scheduleStatus);
	const schedule = $derived(store.config?.schedule);
	let enableError = $state<string | null>(null);
	let enablePending = $state(false);
	let pendingRemoval = $state<
		{ kind: 'rule'; id: string } | { kind: 'quiet'; index: number } | null
	>(null);

	async function toggleEnabled() {
		const next = !(schedule?.enabled ?? false);
		enableError = null;
		enablePending = true;
		try {
			// `store.config.schedule.enabled` only changes once this resolves, so the switch's visual
			// state (driven directly off that value below) never needs an optimistic flip or a revert.
			await store.setScheduleEnabled(next);
			await store.refreshSupervisorStatus();
			taskFeedback.success('schedule-enabled', next ? 'Scheduling enabled' : 'Scheduling disabled');
		} catch (err) {
			enableError = String(err);
			taskFeedback.error('schedule-enabled', `Couldn’t update scheduling: ${String(err)}`);
		} finally {
			enablePending = false;
		}
	}

	function pad(n: number): string {
		return n.toString().padStart(2, '0');
	}

	function toTimeValue(time: TimeOfDay): string {
		return `${pad(time.hour)}:${pad(time.minute)}`;
	}

	function fromTimeValue(value: string): TimeOfDay | null {
		const match = /^(\d{1,2}):(\d{1,2})$/.exec(value);
		if (!match) return null;
		return { hour: Number(match[1]), minute: Number(match[2]) };
	}

	function formatInstant(iso: string | null): string {
		if (!iso) return '';
		const date = new Date(iso);
		const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
		const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
		const today = new Date();
		const tomorrow = new Date(today);
		tomorrow.setDate(today.getDate() + 1);
		if (sameDay(date, today)) return `today ${time}`;
		if (sameDay(date, tomorrow)) return `tomorrow ${time}`;
		return `${date.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })} ${time}`;
	}

	function daySummary(days: boolean[]): string {
		if (days.every(Boolean)) return 'Every day';
		if (days.slice(0, 5).every(Boolean) && !days[5] && !days[6]) return 'Weekdays';
		if (!days.slice(0, 5).some(Boolean) && days[5] && days[6]) return 'Weekends';
		const selected = DAY_LABELS.filter((_, index) => days[index]);
		return selected.length ? selected.join(', ') : 'No days selected';
	}

	function frequencyWord(frequency: Frequency): string {
		const times = frequency.count === 1 ? 'once' : `${frequency.count} times`;
		return `${times} a ${frequency.kind === 'per_day' ? 'day' : 'week'}`;
	}

	function lengthWord(rule: RuleDto): string {
		return rule.length.kind === 'until_stopped'
			? 'until you stop it'
			: `for ${rule.length.minutes} min`;
	}

	// The one readable sentence each rule is meant to reduce to. "About" is not hedging: the rate
	// is capped so it cannot cram a shortfall into the end of a range, so under-delivery is a real
	// (and intended) outcome.
	function ruleSummary(rule: RuleDto): string {
		const days = daySummary(rule.days);
		if (rule.trigger.kind === 'at') {
			return `${days} at ${toTimeValue(rule.trigger.time)}, ${lengthWord(rule)}.`;
		}
		const when =
			rule.trigger.range.kind === 'all_day'
				? 'any time of day'
				: `any time between ${toTimeValue(rule.trigger.range.from)} and ${toTimeValue(rule.trigger.range.to)}`;
		return `${days}, ${when}, about ${frequencyWord(rule.trigger.frequency)}, ${lengthWord(rule)}.`;
	}

	function quietSummary(quiet: QuietHoursDto): string {
		return `${daySummary(quiet.days)}, ${toTimeValue(quiet.start)}–${toTimeValue(quiet.end)}`;
	}

	function toggleDay(days: boolean[], dayIndex: number): boolean[] {
		const next = [...days];
		next[dayIndex] = !next[dayIndex];
		return next;
	}

	type TimingMode = 'at' | 'between' | 'all_day';

	function timingMode(rule: RuleDto): TimingMode {
		if (rule.trigger.kind === 'at') return 'at';
		return rule.trigger.range.kind === 'all_day' ? 'all_day' : 'between';
	}

	// Switching mode keeps whatever times and counts the user already typed, so flipping between
	// "at" and "between" to compare them doesn't quietly discard their work.
	function setTimingMode(rule: RuleDto, mode: TimingMode) {
		if (timingMode(rule) === mode) return;
		const time = rule.trigger.kind === 'at' ? rule.trigger.time : rangeStart(rule);
		const frequency: Frequency =
			rule.trigger.kind === 'rate' ? rule.trigger.frequency : { kind: 'per_day', count: 1 };

		if (mode === 'at') {
			store.updateRule(rule.id, { trigger: { kind: 'at', time } });
		} else if (mode === 'all_day') {
			store.updateRule(rule.id, {
				trigger: { kind: 'rate', range: { kind: 'all_day' }, frequency }
			});
		} else {
			store.updateRule(rule.id, {
				trigger: {
					kind: 'rate',
					range: { kind: 'between', from: time, to: rangeEnd(rule) },
					frequency
				}
			});
		}
	}

	function rangeStart(rule: RuleDto): TimeOfDay {
		if (rule.trigger.kind === 'at') return rule.trigger.time;
		return rule.trigger.range.kind === 'between' ? rule.trigger.range.from : { hour: 9, minute: 0 };
	}

	function rangeEnd(rule: RuleDto): TimeOfDay {
		if (rule.trigger.kind === 'rate' && rule.trigger.range.kind === 'between')
			return rule.trigger.range.to;
		return { hour: 17, minute: 0 };
	}

	function updateRange(rule: RuleDto, patch: { from?: TimeOfDay; to?: TimeOfDay }) {
		if (rule.trigger.kind !== 'rate') return;
		store.updateRule(rule.id, {
			trigger: {
				...rule.trigger,
				range: {
					kind: 'between',
					from: patch.from ?? rangeStart(rule),
					to: patch.to ?? rangeEnd(rule)
				}
			}
		});
	}

	function updateFrequency(rule: RuleDto, patch: Partial<Frequency>) {
		if (rule.trigger.kind !== 'rate') return;
		store.updateRule(rule.id, {
			trigger: { ...rule.trigger, frequency: { ...rule.trigger.frequency, ...patch } as Frequency }
		});
	}

	function setLengthMode(rule: RuleDto, fixed: boolean) {
		if ((rule.length.kind === 'fixed') === fixed) return;
		store.updateRule(rule.id, {
			length: fixed ? { kind: 'fixed', minutes: 20 } : { kind: 'until_stopped' }
		});
	}

	// A rule whose budget has run out of room in the range it draws from. Distinct from
	// `ruleWarning`: that one is about a rule that cannot fire at all, this is about one that fires
	// *less often than it says*, which is the failure a user has no way of noticing on their own --
	// the whole promise of a rate rule is that they cannot predict it, so "that felt like fewer than
	// three" is not evidence of anything. Said when the rule is written, since after that there is
	// nothing to see.
	function crowdingNote(rule: RuleDto, crowding: CrowdingDto): string {
		const asked = rule.trigger.kind === 'rate' ? frequencyWord(rule.trigger.frequency) : '';
		const times = (count: number) => (count === 1 ? 'once' : `${count} times`);
		if (crowding.impossible) {
			const needs = Math.round(crowding.required_minutes);
			const has = Math.round(crowding.available_minutes);
			return `${asked} doesn’t fit here: it needs ${needs} minutes of the ${has} this range has, once the gap between sessions is counted. There’s room for ${times(crowding.max_count)}.`;
		}
		// It fits, but only just. A panic holds Lewdware off for far longer than an ordinary session
		// ending does, so the first one of the day is what breaks a range with no slack in it.
		return `${asked} only just fits this range — one panic would hold sessions off long enough to lose one. ${times(crowding.comfortable_count)} leaves room for that.`;
	}

	// A rule that can never fire. Worth saying out loud rather than leaving the user to wonder why
	// nothing happens.
	function ruleWarning(rule: RuleDto): string | null {
		if (!rule.days.some(Boolean)) return 'Select at least one day for this rule to take effect.';
		if (rule.trigger.kind === 'rate' && rule.trigger.frequency.count === 0)
			return 'A frequency of zero means this rule never runs.';
		return null;
	}

	// Which rules have their pack/mode row open. Collapsed by default: most rules want the global
	// pack and mode, and a rule that does is one line rather than two empty pickers.
	let expandedOverrides = $state(new Set<string>());

	function toggleOverrides(id: string) {
		const next = new Set(expandedOverrides);
		if (!next.delete(id)) next.add(id);
		expandedOverrides = next;
	}

	// Pack-embedded modes are deliberately excluded: their id is a row in *that pack's* table, so
	// pairing one with a different pack override is meaningless. Built-in and uploaded modes work
	// against any pack.
	const overridableModes = $derived(
		store.modeGroups
			.filter((group) => group.source !== 'pack')
			.flatMap((group) => group.entries.map((entry) => ({ entry, group })))
	);

	function modeLabel(mode: ModeId | null): string {
		if (!mode) return 'Default mode';
		const match = overridableModes.find(
			(candidate) => JSON.stringify(candidate.entry.id) === JSON.stringify(mode)
		);
		return match?.entry.name ?? 'Default mode';
	}

	function packLabel(path: string | null): string {
		if (!path) return 'Default pack';
		return path.split(/[\\/]/).pop() || path;
	}

	function overrideSummary(rule: RuleDto): string {
		if (!rule.overrides.pack_path && !rule.overrides.mode) return 'Uses the default pack and mode';
		return `${packLabel(rule.overrides.pack_path)} · ${modeLabel(rule.overrides.mode)}`;
	}

	function setOverrides(rule: RuleDto, patch: Partial<RuleDto['overrides']>) {
		store.updateRule(rule.id, { overrides: { ...rule.overrides, ...patch } });
	}

	async function pickRulePack(rule: RuleDto) {
		const path = await api.pickPackPath().catch(() => null);
		if (path) setOverrides(rule, { pack_path: path });
	}

	function confirmRemoval() {
		const removal = pendingRemoval;
		pendingRemoval = null;
		if (!removal) return;
		if (removal.kind === 'rule') store.removeRule(removal.id);
		else store.removeQuietHours(removal.index);
	}
</script>

{#snippet dayPicker(days: boolean[], label: string, onpick: (dayIndex: number) => void)}
	<div class="flex flex-col gap-1.5">
		<span class="text-text text-xs font-semibold">Days</span>
		<div
			class="border-border inline-flex self-start overflow-hidden rounded-sm border"
			role="group"
			aria-label={label}
		>
			{#each DAY_LABELS as dayLabel, dayIndex (dayIndex)}
				<button
					onclick={() => onpick(dayIndex)}
					aria-pressed={days[dayIndex]}
					class="border-border h-8 w-11 cursor-pointer border-r font-mono text-[11px] font-medium transition-colors last:border-r-0
					{days[dayIndex]
						? 'bg-surface-2 text-text shadow-[inset_0_-2px_0_var(--ui-accent-hover)]'
						: 'bg-bg text-muted hover:text-text'}"
				>
					{dayLabel}
				</button>
			{/each}
		</div>
	</div>
{/snippet}

{#snippet segmented(
	label: string,
	options: { value: string; label: string }[],
	current: string,
	onpick: (value: string) => void
)}
	<div
		class="border-border bg-bg inline-flex self-start rounded-md border p-0.5"
		role="group"
		aria-label={label}
	>
		{#each options as option (option.value)}
			<button
				class="h-7 cursor-pointer rounded-sm px-3 text-xs font-medium transition-colors
				{current === option.value
					? 'bg-surface-2 text-text shadow-[inset_0_-2px_0_var(--ui-accent-hover)]'
					: 'text-muted hover:text-text'}"
				aria-pressed={current === option.value}
				onclick={() => onpick(option.value)}>{option.label}</button
			>
		{/each}
	</div>
{/snippet}

<div class="min-h-0 flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Scheduling</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Start sessions on their own, at a set time or an unpredictable number of times within a
				range. Lewdware runs a background supervisor at login while scheduling is enabled.
			</p>
		</header>

		<section class="flex flex-col gap-3">
			<Card class="flex items-center gap-4 p-4">
				<div class="min-w-0 flex-1">
					<h2 class="text-text m-0 text-sm font-semibold">Automatic scheduling</h2>
					<p class="text-muted m-0 mt-1 font-mono text-[11px]">
						{#if !schedule?.enabled}
							Disabled. No sessions will start automatically.
						{:else if status.cooldown_until}
							Paused until {formatInstant(status.cooldown_until)}
						{:else if status.next_exact_session}
							Next session: {formatInstant(status.next_exact_session)}
						{:else if status.budget_total > 0}
							{status.budget_remaining} of {status.budget_total} left in this period{#if status.next_opportunity}{' '}·
								not before {formatInstant(status.next_opportunity)}{/if}
						{:else}
							Enabled, but no rule can start a session yet.
						{/if}
					</p>
				</div>
				{#if status.cooldown_until}
					<Button size="compact" variant="secondary" onclick={() => store.resumeSchedule()}
						>Resume now</Button
					>
				{/if}
				<span class="text-xs font-medium {schedule?.enabled ? 'text-text' : 'text-muted'}">
					{schedule?.enabled ? 'Enabled' : 'Disabled'}
				</span>
				<Toggle
					ariaLabel="Enable scheduling"
					checked={schedule?.enabled ?? false}
					disabled={enablePending}
					onchange={() => toggleEnabled()}
				/>
			</Card>

			{#if enableError}
				<div
					class="flex items-center gap-3 rounded-md border border-[var(--ui-danger-border)] bg-[var(--ui-danger-bg)] px-3 py-2 text-sm text-[var(--ui-danger)]"
				>
					<span>Couldn't update scheduling: {enableError}</span>
				</div>
			{/if}
		</section>

		{#if schedule?.enabled}
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<div class="flex items-start justify-between gap-4">
					<div>
						<h2 class="ui-section-title">Rules</h2>
						<p class="text-muted m-0 mt-1 text-xs">
							Each rule decides when a session may start, how often, and how long it runs.
						</p>
					</div>
					<Button size="compact" variant="secondary" onclick={() => store.addRule()}
						>Add rule</Button
					>
				</div>
				<div class="flex flex-col gap-3">
					{#each schedule.rules as rule, i (rule.id)}
						{@const mode = timingMode(rule)}
						{@const warning = ruleWarning(rule)}
						{@const crowding = store.crowding.find((c) => c.rule_id === rule.id)}
						<Card class="flex flex-col gap-4 p-4">
							<div class="flex items-start justify-between gap-4">
								<div class="min-w-0">
									<h3 class="text-text m-0 text-sm font-semibold">Rule {i + 1}</h3>
									<p class="text-muted m-0 mt-1 font-mono text-[11px]">{ruleSummary(rule)}</p>
								</div>
								<Button
									size="compact"
									variant="destructive"
									onclick={() => (pendingRemoval = { kind: 'rule', id: rule.id })}>Remove</Button
								>
							</div>

							{@render dayPicker(rule.days, `Days for rule ${i + 1}`, (dayIndex) =>
								store.updateRule(rule.id, { days: toggleDay(rule.days, dayIndex) })
							)}

							<div class="flex flex-col gap-2">
								<span class="text-text text-xs font-semibold">When</span>
								{@render segmented(
									`Timing for rule ${i + 1}`,
									[
										{ value: 'at', label: 'At a set time' },
										{ value: 'between', label: 'Between times' },
										{ value: 'all_day', label: 'Any time of day' }
									],
									mode,
									(value) => setTimingMode(rule, value as TimingMode)
								)}
								<p class="text-muted text-xs">
									{#if mode === 'at'}
										Starts at exactly this time, whether or not you’re at your computer.
									{:else}
										Starts at an unpredictable moment, chosen while you’re actually using the
										computer. Lewdware won’t tell you when.
									{/if}
								</p>
							</div>

							{#if mode !== 'all_day'}
								<div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
									{#if mode === 'at'}
										<Field
											label="Time"
											type="time"
											size="compact"
											value={toTimeValue(rangeStart(rule))}
											onchange={(value) => {
												const time = fromTimeValue(value);
												if (time) store.updateRule(rule.id, { trigger: { kind: 'at', time } });
											}}
										/>
									{:else if mode === 'between'}
										<Field
											label="From"
											type="time"
											size="compact"
											value={toTimeValue(rangeStart(rule))}
											onchange={(value) => {
												const from = fromTimeValue(value);
												if (from) updateRange(rule, { from });
											}}
										/>
										<Field
											label="Until"
											type="time"
											size="compact"
											value={toTimeValue(rangeEnd(rule))}
											onchange={(value) => {
												const to = fromTimeValue(value);
												if (to) updateRange(rule, { to });
											}}
										/>
									{/if}
								</div>
							{/if}

							{#if rule.trigger.kind === 'rate'}
								{@const frequency = rule.trigger.frequency}
								<div class="flex flex-col gap-1.5">
									<span class="text-text flex items-center gap-1 text-xs font-semibold">
										How often
										<Popover label="About the number of sessions" role="dialog">
											{#snippet trigger(toggle, open)}
												<IconButton
													label="About the number of sessions"
													ariaHaspopup="dialog"
													ariaExpanded={open}
													class="!h-5 !w-5"
													onclick={toggle}
												>
													<Icon src={QuestionMarkCircle} mini size="14px" />
												</IconButton>
											{/snippet}
											{#snippet children()}
												<p class="text-muted m-0 max-w-[264px] p-3 text-xs leading-relaxed">
													{RATE_EXPLANATION}
												</p>
											{/snippet}
										</Popover>
									</span>
									<div class="flex items-center gap-2">
										<NumberField
											label="How many sessions"
											hideLabel
											size="compact"
											class="w-24"
											min={1}
											max={99}
											value={frequency.count}
											onchange={(count) => {
												// An empty field is mid-edit, not a frequency of zero.
												if (count === null) return;
												updateFrequency(rule, { count: Math.max(1, count) });
											}}
										/>
										<Select
											label="How often"
											hideLabel
											size="compact"
											class="w-40"
											value={frequency.kind}
											options={[
												{ value: 'per_day', label: 'times a day' },
												{ value: 'per_week', label: 'times a week' }
											]}
											onchange={(value) =>
												updateFrequency(rule, { kind: value as Frequency['kind'] })}
										/>
									</div>
								</div>
							{/if}

							<div class="flex flex-col gap-2">
								<span class="text-text text-xs font-semibold">Session length</span>
								{@render segmented(
									`Session length for rule ${i + 1}`,
									[
										{ value: 'until_stopped', label: 'Until I stop it' },
										{ value: 'fixed', label: 'For a set time' }
									],
									rule.length.kind,
									(value) => setLengthMode(rule, value === 'fixed')
								)}
								{#if rule.length.kind === 'fixed'}
									<div class="grid grid-cols-2 gap-3 sm:grid-cols-3">
										<NumberField
											label="Minutes"
											size="compact"
											min={1}
											max={1440}
											value={rule.length.minutes}
											onchange={(minutes) => {
												if (minutes === null) return;
												store.updateRule(rule.id, {
													length: { kind: 'fixed', minutes: Math.max(1, minutes) }
												});
											}}
										/>
									</div>
								{:else}
									<p class="text-muted text-xs">
										Runs until you stop it or quiet hours begin. It also ends on its own if you
										leave the computer for a while, so nothing is left running at an empty desk.
									</p>
								{/if}
							</div>

							<div class="border-border flex flex-col gap-2 border-t pt-3">
								<button
									class="text-muted hover:text-text flex cursor-pointer items-center gap-1.5 self-start text-xs transition-colors"
									aria-expanded={expandedOverrides.has(rule.id)}
									onclick={() => toggleOverrides(rule.id)}
								>
									<span class="font-mono text-[11px]">{overrideSummary(rule)}</span>
									<span aria-hidden="true">{expandedOverrides.has(rule.id) ? '▴' : '▾'}</span>
								</button>
								{#if expandedOverrides.has(rule.id)}
									<div class="grid gap-3 sm:grid-cols-2">
										<div class="flex flex-col gap-1.5">
											<span class="text-text text-xs font-semibold">Pack</span>
											<div class="flex items-center gap-2">
												<Button
													size="compact"
													variant="secondary"
													onclick={() => pickRulePack(rule)}>Choose pack…</Button
												>
												{#if rule.overrides.pack_path}
													<Button
														size="compact"
														variant="quiet"
														onclick={() => setOverrides(rule, { pack_path: null })}
														>Use default</Button
													>
												{/if}
											</div>
											<span class="text-muted truncate font-mono text-[11px]"
												>{packLabel(rule.overrides.pack_path)}</span
											>
										</div>
										<Select
											label="Mode"
											size="compact"
											value={rule.overrides.mode ? JSON.stringify(rule.overrides.mode) : ''}
											options={[
												{ value: '', label: 'Default mode' },
												...overridableModes.map(({ entry, group }) => ({
													value: JSON.stringify(entry.id),
													label: entry.name,
													description: group.label
												}))
											]}
											onchange={(value) =>
												setOverrides(rule, { mode: value ? (JSON.parse(value) as ModeId) : null })}
										/>
									</div>
								{/if}
							</div>

							{#if warning}
								<span class="text-xs text-[var(--ui-warning)]">{warning}</span>
							{:else if crowding}
								<span class="text-xs text-[var(--ui-warning)]">{crowdingNote(rule, crowding)}</span>
							{/if}
						</Card>
					{/each}
					{#if schedule.rules.length === 0}
						<Card
							class="flex flex-col items-center border-dashed !border-[var(--ui-border-strong)] p-7 text-center"
						>
							<h3 class="text-text m-0 text-sm font-semibold">No rules</h3>
							<p class="text-muted m-0 mt-1 mb-4 text-xs">
								Scheduling is enabled, but no sessions can start until you add a rule.
							</p>
							<Button size="compact" variant="secondary" onclick={() => store.addRule()}
								>Add rule</Button
							>
						</Card>
					{/if}
				</div>
			</section>

			<!-- Quiet hours -->
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<div class="flex items-start justify-between gap-4">
					<div>
						<h2 class="ui-section-title">Quiet hours</h2>
						<p class="text-muted m-0 mt-1 text-xs">
							Never start during these times, and stop a scheduled session that is already running.
							Sessions you launch yourself are unaffected.
						</p>
					</div>
					<Button size="compact" variant="secondary" onclick={() => store.addQuietHours()}
						>Add quiet hours</Button
					>
				</div>
				<div class="flex flex-col gap-3">
					{#each schedule.quiet_hours as quiet, i (i)}
						<Card class="flex flex-col gap-4 p-4">
							<div class="flex items-start justify-between gap-4">
								<div>
									<h3 class="text-text m-0 text-sm font-semibold">Quiet period {i + 1}</h3>
									<p class="text-muted m-0 mt-1 font-mono text-[11px]">{quietSummary(quiet)}</p>
								</div>
								<Button
									size="compact"
									variant="destructive"
									onclick={() => (pendingRemoval = { kind: 'quiet', index: i })}>Remove</Button
								>
							</div>
							{@render dayPicker(quiet.days, `Days for quiet period ${i + 1}`, (dayIndex) =>
								store.updateQuietHours(i, { days: toggleDay(quiet.days, dayIndex) })
							)}
							<div class="grid grid-cols-2 gap-3">
								<Field
									label="From"
									type="time"
									size="compact"
									value={toTimeValue(quiet.start)}
									onchange={(value) => {
										const start = fromTimeValue(value);
										if (start) store.updateQuietHours(i, { start });
									}}
								/>
								<Field
									label="Until"
									type="time"
									size="compact"
									value={toTimeValue(quiet.end)}
									onchange={(value) => {
										const end = fromTimeValue(value);
										if (end) store.updateQuietHours(i, { end });
									}}
								/>
							</div>
							{#if !quiet.days.some(Boolean)}
								<span class="text-xs text-[var(--ui-warning)]"
									>Select at least one day for this quiet period to take effect.</span
								>
							{:else if quiet.start.hour === quiet.end.hour && quiet.start.minute === quiet.end.minute}
								<span class="text-xs text-[var(--ui-warning)]"
									>Start and end are the same, so this quiet period has no effect.</span
								>
							{/if}
						</Card>
					{/each}
					{#if schedule.quiet_hours.length === 0}
						<Card class="border-dashed !border-[var(--ui-border-strong)] p-5 text-center">
							<p class="text-muted m-0 text-xs">
								No quiet hours. Scheduled sessions may run whenever a rule allows.
							</p>
						</Card>
					{/if}
				</div>
			</section>

			<!-- Pacing -->
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<h2 class="ui-section-title">Pacing</h2>
				<p class="text-muted text-xs">
					How scheduled sessions space themselves out, and how long a panic holds them off.
				</p>
				<Card class="flex items-center gap-4 p-4">
					<div class="min-w-0 flex-1">
						<h3 class="text-text m-0 text-sm font-medium">Minimum gap between sessions</h3>
						<p class="text-muted m-0 mt-1 text-xs">
							Stops “3 times a day” from arriving as three in a row.
						</p>
					</div>
					<NumberField
						label="Minimum gap between sessions"
						hideLabel
						size="compact"
						class="w-28"
						min={1}
						max={1440}
						suffix="min"
						value={schedule.cooldown_minutes}
						onchange={(minutes) => {
							if (minutes === null) return;
							store.setScheduleSettings({ cooldown_minutes: Math.max(1, minutes) });
						}}
					/>
				</Card>
				<Card class="flex items-center gap-4 p-4">
					<div class="min-w-0 flex-1">
						<h3 class="text-text m-0 text-sm font-medium">Pause after a panic</h3>
						<p class="text-muted m-0 mt-1 text-xs">
							The panic key always stops the running session. This is how long it also stops
							scheduled ones from starting again — press it with nothing running to pause
							pre-emptively.
						</p>
					</div>
					<Select
						label="Pause after a panic"
						hideLabel
						size="compact"
						class="w-40"
						value={String(schedule.panic_cooldown_minutes)}
						options={PANIC_COOLDOWNS}
						onchange={(value) =>
							store.setScheduleSettings({ panic_cooldown_minutes: Number(value) })}
					/>
				</Card>
			</section>

			<!-- Grace notification -->
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<h2 class="ui-section-title">Grace notification</h2>
				<p class="text-muted text-xs">
					A few seconds’ desktop notification before a scheduled session starts, with a Cancel
					action.
				</p>
				<Card class="flex items-center justify-between gap-4 p-4">
					<div>
						<h3 class="text-text m-0 text-sm font-medium">Warn before starting</h3>
						<p class="text-muted m-0 mt-1 text-xs">
							Cancelling skips that session and starts the gap above.
						</p>
					</div>
					<Toggle
						checked={schedule.grace_notification}
						ariaLabel="Warn before a scheduled session starts"
						onchange={(checked) => store.setGraceNotification(checked)}
					/>
				</Card>
			</section>
		{:else}
			<Card
				class="flex flex-col items-center border-dashed !border-[var(--ui-border-strong)] p-8 text-center"
			>
				<h2 class="text-text m-0 text-sm font-semibold">Scheduling is turned off</h2>
				<p class="text-muted m-0 mt-1 max-w-md text-xs leading-relaxed">
					Enable automatic scheduling above to review and edit rules, quiet hours, pacing, and
					startup notifications.
				</p>
			</Card>
		{/if}
	</div>
</div>

{#if pendingRemoval}
	<Dialog
		title={pendingRemoval.kind === 'rule' ? 'Remove rule?' : 'Remove quiet period?'}
		description={pendingRemoval.kind === 'rule'
			? 'Sessions will no longer start from this rule.'
			: 'Scheduled activity will no longer be blocked during this quiet period.'}
		buttons={[
			{ label: 'Cancel', onclick: () => (pendingRemoval = null) },
			{ label: 'Remove', destructive: true, onclick: confirmRemoval }
		]}
		onclose={() => (pendingRemoval = null)}
	/>
{/if}
