<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { store } from './store.svelte';
	import Toggle from '$ui/Toggle.svelte';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import Dialog from '$ui/Dialog.svelte';
	import Field from '$ui/Field.svelte';
	import NumberField from '$ui/NumberField.svelte';
	import type { QuietHoursDto, WindowDto } from './types';
	import { taskFeedback } from '$ui/taskFeedback.svelte.js';

	const DAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];

	// Kept fresh by the `supervisor:status` push event; see +page.svelte.
	const status = $derived(store.scheduleStatus);
	let enableError = $state<string | null>(null);
	let enablePending = $state(false);
	let pendingRemoval = $state<{ kind: 'window' | 'quiet'; index: number } | null>(null);

	async function toggleEnabled() {
		const next = !(store.config?.schedule.enabled ?? false);
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

	function toTimeValue(hour: number, minute: number): string {
		return `${pad(hour)}:${pad(minute)}`;
	}

	function fromTimeValue(value: string): { hour: number; minute: number } | null {
		const match = /^(\d{1,2}):(\d{1,2})$/.exec(value);
		if (!match) return null;
		return { hour: Number(match[1]), minute: Number(match[2]) };
	}

	function formatNextSession(iso: string | null): string {
		if (!iso) return '';
		const date = new Date(iso);
		const time = date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });
		const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
		const today = new Date();
		const tomorrow = new Date(today);
		tomorrow.setDate(today.getDate() + 1);
		if (sameDay(date, today)) return `today - ${time}`;
		if (sameDay(date, tomorrow)) return `tomorrow - ${time}`;
		return `${date.toLocaleDateString(undefined, { weekday: 'short', day: 'numeric', month: 'short' })} - ${time}`;
	}

	function toggleWindowDay(index: number, window: WindowDto, dayIndex: number) {
		const days = [...window.days];
		days[dayIndex] = !days[dayIndex];
		store.updateWindow(index, { days });
	}

	function toggleQuietDay(index: number, quiet: QuietHoursDto, dayIndex: number) {
		const days = [...quiet.days];
		days[dayIndex] = !days[dayIndex];
		store.updateQuietHours(index, { days });
	}

	function daySummary(days: boolean[]): string {
		if (days.every(Boolean)) return 'Every day';
		if (days.slice(0, 5).every(Boolean) && !days[5] && !days[6]) return 'Weekdays';
		if (!days.slice(0, 5).some(Boolean) && days[5] && days[6]) return 'Weekends';
		const selected = DAY_LABELS.filter((_, index) => days[index]);
		return selected.length ? selected.join(', ') : 'No days selected';
	}

	function windowSummary(window: WindowDto): string {
		const start = toTimeValue(window.start_hour, window.start_minute);
		const timing =
			window.jitter_minutes > 0 ? `between ${start} and ${windowEndValue(window)}` : `at ${start}`;
		return `${daySummary(window.days)}, ${timing}, for ${window.duration_minutes} min`;
	}

	function windowEndValue(window: WindowDto): string {
		const total = (window.start_hour * 60 + window.start_minute + window.jitter_minutes) % 1440;
		return toTimeValue(Math.floor(total / 60), total % 60);
	}

	function setWindowStartMode(index: number, window: WindowDto, between: boolean) {
		store.updateWindow(index, { jitter_minutes: between ? window.jitter_minutes || 30 : 0 });
	}

	function updateWindowEnd(index: number, window: WindowDto, value: string) {
		const end = fromTimeValue(value);
		if (!end) return;
		const startMinutes = window.start_hour * 60 + window.start_minute;
		const endMinutes = end.hour * 60 + end.minute;
		const difference = (endMinutes - startMinutes + 1440) % 1440;
		// Equal clock times represent a full-day range rather than unexpectedly switching the UI
		// back to “Start at”.
		store.updateWindow(index, { jitter_minutes: difference || 1440 });
	}

	function quietSummary(quiet: QuietHoursDto): string {
		return `${daySummary(quiet.days)}, ${toTimeValue(quiet.start_hour, quiet.start_minute)}–${toTimeValue(quiet.end_hour, quiet.end_minute)}`;
	}

	function confirmRemoval() {
		const removal = pendingRemoval;
		pendingRemoval = null;
		if (!removal) return;
		if (removal.kind === 'window') store.removeWindow(removal.index);
		else store.removeQuietHours(removal.index);
	}
</script>

<div class="flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">Scheduling</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Start sessions automatically within allowed windows. Lewdware runs a background supervisor
				at login while scheduling is enabled.
			</p>
		</header>

		<section class="flex flex-col gap-3">
			<Card class="flex items-center gap-4 p-4">
				<div class="min-w-0 flex-1">
					<h2 class="text-text m-0 text-sm font-semibold">Automatic scheduling</h2>
					<p class="text-muted m-0 mt-1 text-xs">
						{#if store.config?.schedule.enabled && status.next_session}
							<span class="font-mono text-[11px]"
								>Next session: {formatNextSession(status.next_session)}</span
							>
						{:else if store.config?.schedule.enabled}
							Enabled, but no upcoming session matches the current rules.
						{:else}
							Disabled. No sessions will start automatically.
						{/if}
					</p>
				</div>
				<span
					class="text-xs font-medium {store.config?.schedule.enabled ? 'text-text' : 'text-muted'}"
				>
					{store.config?.schedule.enabled ? 'Enabled' : 'Disabled'}
				</span>
				<Toggle
					ariaLabel="Enable scheduling"
					checked={store.config?.schedule.enabled ?? false}
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

		{#if store.config?.schedule.enabled}
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<div class="flex items-start justify-between gap-4">
					<div>
						<h2 class="ui-section-title">Session windows</h2>
						<p class="text-muted m-0 mt-1 text-xs">
							Define when a session may start, how long it runs, and whether its start time varies.
						</p>
					</div>
					<Button size="compact" variant="secondary" onclick={() => store.addWindow()}
						>Add window</Button
					>
				</div>
				<div class="flex flex-col gap-3">
					{#each store.config?.schedule.windows ?? [] as window, i (i)}
						<Card class="flex flex-col gap-4 p-4">
							<div class="flex items-start justify-between gap-4">
								<div class="min-w-0">
									<h3 class="text-text m-0 text-sm font-semibold">Window {i + 1}</h3>
									<p class="text-muted m-0 mt-1 text-xs">{windowSummary(window)}</p>
								</div>
								<Button
									size="compact"
									variant="destructive"
									onclick={() => (pendingRemoval = { kind: 'window', index: i })}>Remove</Button
								>
							</div>
							<div class="flex flex-col gap-1.5">
								<span class="text-text text-xs font-semibold">Days</span>
								<div
									class="border-border inline-flex self-start overflow-hidden rounded-sm border"
									role="group"
									aria-label={`Days for window ${i + 1}`}
								>
									{#each DAY_LABELS as label, dayIndex (dayIndex)}
										<button
											onclick={() => toggleWindowDay(i, window, dayIndex)}
											aria-pressed={window.days[dayIndex]}
											class="border-border h-8 w-11 cursor-pointer border-r font-mono text-[11px] font-medium transition-colors last:border-r-0
                       {window.days[dayIndex]
												? 'bg-surface-2 text-text shadow-[inset_0_-2px_0_var(--ui-accent-hover)]'
												: 'bg-bg text-muted hover:text-text'}"
										>
											{label}
										</button>
									{/each}
								</div>
								{#if !window.days.some(Boolean)}<span class="text-xs text-[var(--ui-warning)]"
										>Select at least one day for this window to take effect.</span
									>{/if}
							</div>
							<div class="flex flex-col gap-2">
								<span class="text-text text-xs font-semibold">Start timing</span>
								<div
									class="border-border bg-bg inline-flex self-start rounded-md border p-0.5"
									role="group"
									aria-label={`Start timing for window ${i + 1}`}
								>
									<button
										class="h-7 cursor-pointer rounded px-3 text-xs font-medium transition-colors {window.jitter_minutes ===
										0
											? 'bg-surface-2 text-text'
											: 'text-muted hover:text-text'}"
										aria-pressed={window.jitter_minutes === 0}
										onclick={() => setWindowStartMode(i, window, false)}>Start at</button
									>
									<button
										class="h-7 cursor-pointer rounded px-3 text-xs font-medium transition-colors {window.jitter_minutes >
										0
											? 'bg-surface-2 text-text'
											: 'text-muted hover:text-text'}"
										aria-pressed={window.jitter_minutes > 0}
										onclick={() => setWindowStartMode(i, window, true)}>Start between</button
									>
								</div>
							</div>
							{#if window.jitter_minutes > 0}
								<p class="text-muted text-xs">
									{window.jitter_minutes === 1440
										? 'Any time during the following 24 hours:'
										: 'The session starts at a random time in this range.'}
								</p>
							{/if}
							<div class="grid gap-3 {window.jitter_minutes > 0 ? 'grid-cols-3' : 'grid-cols-2'}">
								<Field
									label={window.jitter_minutes > 0 ? 'From' : 'Start time'}
									type="time"
									size="compact"
									value={toTimeValue(window.start_hour, window.start_minute)}
									onchange={(value) => {
										const t = fromTimeValue(value);
										if (t) store.updateWindow(i, { start_hour: t.hour, start_minute: t.minute });
									}}
								/>
								{#if window.jitter_minutes > 0}
									<Field
										label="Until"
										type="time"
										size="compact"
										value={windowEndValue(window)}
										onchange={(value) => updateWindowEnd(i, window, value)}
									/>
								{/if}
								<NumberField
									label="Duration (minutes)"
									size="compact"
									min={1}
									max={1440}
									value={window.duration_minutes}
									onchange={(minutes) => {
										// An empty field is mid-edit, not a zero-length window. It used to read as
										// one: `Number('')` is 0, so clearing the box silently rewrote the window
										// to a single minute.
										if (minutes === null) return;
										store.updateWindow(i, { duration_minutes: Math.max(1, minutes) });
									}}
								/>
							</div>
						</Card>
					{/each}
					{#if (store.config?.schedule.windows ?? []).length === 0}
						<Card
							class="flex flex-col items-center border-dashed !border-[var(--ui-border-strong)] p-7 text-center"
						>
							<h3 class="text-text m-0 text-sm font-semibold">No session windows</h3>
							<p class="text-muted m-0 mt-1 mb-4 text-xs">
								Scheduling is enabled, but no sessions can start until you add a window.
							</p>
							<Button size="compact" variant="secondary" onclick={() => store.addWindow()}
								>Add window</Button
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
							Prevent scheduled activity during these times. Manually launched sessions are
							unaffected.
						</p>
					</div>
					<Button size="compact" variant="secondary" onclick={() => store.addQuietHours()}
						>Add quiet hours</Button
					>
				</div>
				<div class="flex flex-col gap-3">
					{#each store.config?.schedule.quiet_hours ?? [] as quiet, i (i)}
						<Card class="flex flex-col gap-4 p-4">
							<div class="flex items-start justify-between gap-4">
								<div>
									<h3 class="text-text m-0 text-sm font-semibold">Quiet period {i + 1}</h3>
									<p class="text-muted m-0 mt-1 text-xs">{quietSummary(quiet)}</p>
								</div>
								<Button
									size="compact"
									variant="destructive"
									onclick={() => (pendingRemoval = { kind: 'quiet', index: i })}>Remove</Button
								>
							</div>
							<div class="flex flex-col gap-1.5">
								<span class="text-text text-xs font-semibold">Days</span>
								<div
									class="border-border inline-flex self-start overflow-hidden rounded-sm border"
									role="group"
									aria-label={`Days for quiet period ${i + 1}`}
								>
									{#each DAY_LABELS as label, dayIndex (dayIndex)}
										<button
											onclick={() => toggleQuietDay(i, quiet, dayIndex)}
											aria-pressed={quiet.days[dayIndex]}
											class="border-border h-8 w-11 cursor-pointer border-r font-mono text-[11px] font-medium transition-colors last:border-r-0
                       {quiet.days[dayIndex]
												? 'bg-surface-2 text-text shadow-[inset_0_-2px_0_var(--ui-accent-hover)]'
												: 'bg-bg text-muted hover:text-text'}">{label}</button
										>
									{/each}
								</div>
								{#if !quiet.days.some(Boolean)}<span class="text-xs text-[var(--ui-warning)]"
										>Select at least one day for this quiet period to take effect.</span
									>{/if}
							</div>
							<div class="grid grid-cols-2 gap-3">
								<Field
									label="From"
									type="time"
									size="compact"
									value={toTimeValue(quiet.start_hour, quiet.start_minute)}
									onchange={(value) => {
										const t = fromTimeValue(value);
										if (t)
											store.updateQuietHours(i, { start_hour: t.hour, start_minute: t.minute });
									}}
								/><Field
									label="Until"
									type="time"
									size="compact"
									value={toTimeValue(quiet.end_hour, quiet.end_minute)}
									onchange={(value) => {
										const t = fromTimeValue(value);
										if (t) store.updateQuietHours(i, { end_hour: t.hour, end_minute: t.minute });
									}}
								/>
							</div>
							{#if quiet.start_hour === quiet.end_hour && quiet.start_minute === quiet.end_minute}<span
									class="text-xs text-[var(--ui-warning)]"
									>Start and end are the same, so this quiet period has no effect.</span
								>{/if}
						</Card>
					{/each}
					{#if (store.config?.schedule.quiet_hours ?? []).length === 0}
						<Card class="border-dashed !border-[var(--ui-border-strong)] p-5 text-center">
							<p class="text-muted m-0 text-xs">
								No quiet hours. Scheduled sessions may run during any configured window.
							</p>
						</Card>
					{/if}
				</div>
			</section>

			<!-- Grace notification -->
			<section class="border-border flex flex-col gap-3 border-t pt-6">
				<h2 class="ui-section-title">Grace notification</h2>
				<p class="text-muted text-xs">
					A short desktop notification before a scheduled session starts, with a Cancel action that
					skips just that one occurrence.
				</p>
				<Card class="flex items-center justify-between gap-4 p-4"
					><div>
						<h3 class="text-text m-0 text-sm font-medium">Warn before starting</h3>
						<p class="text-muted m-0 mt-1 text-xs">
							The notification includes an action to skip that occurrence.
						</p>
					</div>
					<Toggle
						checked={store.config?.schedule.grace_notification ?? false}
						ariaLabel="Warn before a scheduled session starts"
						onchange={(checked) => store.setGraceNotification(checked)}
					/></Card
				>
			</section>
		{:else}
			<Card
				class="flex flex-col items-center border-dashed !border-[var(--ui-border-strong)] p-8 text-center"
			>
				<h2 class="text-text m-0 text-sm font-semibold">Scheduling is turned off</h2>
				<p class="text-muted m-0 mt-1 max-w-md text-xs leading-relaxed">
					Enable automatic scheduling above to review and edit session windows, quiet hours, and
					startup notifications.
				</p>
			</Card>
		{/if}
	</div>
</div>

{#if pendingRemoval}
	<Dialog
		title={pendingRemoval.kind === 'window' ? 'Remove session window?' : 'Remove quiet period?'}
		description={pendingRemoval.kind === 'window'
			? 'Sessions will no longer be able to start during this window.'
			: 'Scheduled activity will no longer be blocked during this quiet period.'}
		buttons={[
			{ label: 'Cancel', onclick: () => (pendingRemoval = null) },
			{ label: 'Remove', destructive: true, onclick: confirmRemoval }
		]}
		onclose={() => (pendingRemoval = null)}
	/>
{/if}
