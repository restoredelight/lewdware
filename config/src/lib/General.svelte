<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from './api';
	import { store } from './store.svelte';
	import type { Key } from './types';
	import Checkbox from '$ui/Checkbox.svelte';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import { taskFeedback } from './taskFeedback.svelte';

	// Rejections from the launch command itself; engine-level errors (a crash after a successful
	// launch, giving up on restarts) arrive via the pushed store.engineStatus.
	let launchError = $state<string | null>(null);
	let inputMonitoringGranted = $state(true);
	let inputMonitoringPromptFailed = $state(false);
	let engineAction = $state<'launch' | 'stop' | null>(null);

	const engineStatus = $derived(store.engineStatus);
	const running = $derived(engineStatus.running);

	async function checkInputMonitoringGranted() {
		try {
			inputMonitoringGranted = await api.inputMonitoringGranted();
		} catch (err) {
			taskFeedback.warning(
				'input-monitoring',
				`Couldn’t check Input Monitoring permission: ${String(err)}`
			);
		}
	}

	onMount(async () => {
		await checkInputMonitoringGranted();
	});

	async function launch() {
		engineAction = 'launch';
		launchError = null;
		taskFeedback.progress('engine', 'Launching Lewdware…');
		try {
			if (!(await store.saveConfig())) throw new Error('settings could not be saved');
			await api.launchLewdware();
			// The push subscription may take a beat to reach a freshly spawned supervisor.
			void store.refreshSupervisorStatus();
			taskFeedback.success('engine', 'Lewdware launched');
		} catch (err) {
			launchError = String(err);
			taskFeedback.error('engine', `Lewdware failed to launch: ${String(err)}`);
		} finally {
			engineAction = null;
		}
	}

	async function stop() {
		engineAction = 'stop';
		launchError = null;
		taskFeedback.progress('engine', 'Stopping Lewdware…');
		try {
			await api.stopLewdware();
			taskFeedback.success('engine', 'Lewdware stopped');
		} catch (err) {
			taskFeedback.error('engine', `Lewdware couldn’t be stopped: ${String(err)}`);
		} finally {
			engineAction = null;
		}
	}

	async function openInputMonitoringSettings() {
		try {
			const granted = await api.requestInputMonitoring();
			if (granted) {
				inputMonitoringGranted = true;
				taskFeedback.dismiss('input-monitoring');
			} else {
				inputMonitoringPromptFailed = true;
			}
		} catch (err) {
			inputMonitoringPromptFailed = true;
			taskFeedback.error(
				'input-monitoring',
				`Couldn’t request Input Monitoring permission: ${String(err)}`
			);
		}
	}

	async function openLogs() {
		taskFeedback.progress('logs', 'Opening logs folder…');
		try {
			await api.openLogs();
			taskFeedback.success('logs', 'Logs folder opened');
		} catch (err) {
			taskFeedback.error('logs', `Couldn’t open logs folder: ${String(err)}`);
		}
	}

	let recording = $state(false);

	const panicKeyDisplay = $derived(
		recording ? 'Press a key…' : store.config ? formatKey(store.config.panic_button) : ''
	);

	const hasPack = $derived(!!store.config?.pack_path);

	function formatKey(key: Key): string {
		const parts: string[] = [];
		if (key.modifiers.ctrl) parts.push('Ctrl');
		if (key.modifiers.alt) parts.push('Alt');
		if (key.modifiers.shift) parts.push('Shift');
		if (key.modifiers.meta) parts.push('Meta');
		parts.push(key.name);
		return parts.join(' + ');
	}

	const MODIFIER_KEYS = new Set(['Control', 'Alt', 'Shift', 'Meta', 'Super', 'Hyper']);

	function handleKeyDown(e: KeyboardEvent) {
		if (!recording) return;
		if (MODIFIER_KEYS.has(e.key)) return;

		e.preventDefault();

		store.setPanicButton({
			name: e.key === ' ' ? 'Space' : e.key,
			code: e.code,
			modifiers: {
				ctrl: e.ctrlKey,
				alt: e.altKey,
				shift: e.shiftKey,
				meta: e.metaKey
			}
		} satisfies Key);

		recording = false;
	}
</script>

<div class="flex-1 overflow-y-auto">
	<div class="mx-auto flex w-full max-w-4xl flex-col gap-6 p-8">
		<header class="max-w-2xl">
			<h1 class="ui-page-title">General</h1>
			<p class="text-muted mt-1.5 mb-0 text-sm">
				Start or stop Lewdware and configure the controls and displays it uses.
			</p>
		</header>

		<!-- Launch / Stop -->
		<section class="flex flex-col gap-2">
			<h2 class="ui-section-title">Session</h2>
			<p class="text-muted m-0 text-xs">
				Launch Lewdware with the currently selected pack and mode.
			</p>
			<Card class="flex items-center gap-4 p-4">
				<span
					class="h-2.5 w-2.5 shrink-0 rounded-full {running
						? 'bg-accent'
						: 'bg-muted'} {engineAction ? 'animate-pulse' : ''}"
				></span>
				<div class="min-w-0 flex-1">
					<h3 class="text-text m-0 text-sm font-semibold">
						{running ? 'Lewdware is running' : 'Lewdware is stopped'}
					</h3>
					<p class="text-muted m-0 mt-1 text-xs">
						{running
							? 'The current session is active.'
							: hasPack
								? 'Ready to launch.'
								: 'Select a pack before launching.'}
					</p>
				</div>
				{#if running}
					<Button
						size="compact"
						variant="destructive"
						onclick={stop}
						loading={engineAction === 'stop'}>Stop session</Button
					>
				{:else}
					<Button
						size="compact"
						variant="primary"
						onclick={launch}
						disabled={!hasPack}
						loading={engineAction === 'launch'}
						title={hasPack ? undefined : 'Select a pack'}>Launch</Button
					>
				{/if}
			</Card>
			<!-- {#if !hasPack && !running} -->
			<!--   <div class="flex items-center gap-3 px-3 py-2 rounded-md bg-[var(--ui-warning-bg)] border border-[var(--ui-warning-border)] text-sm text-[var(--ui-warning)]"> -->
			<!--     <span>No pack selected. Upload a pack to launch Lewdware.</span> -->
			<!--     <Button size="compact" variant="secondary" class="ml-auto" onclick={() => (store.activeTab = "pack_mode")}>Choose pack</Button> -->
			<!--   </div> -->
			<!-- {/if} -->
			{#if !running && (launchError || engineStatus.error)}
				<div
					class="flex items-center gap-3 rounded-md border border-[var(--ui-danger-border)] bg-[var(--ui-danger-bg)] px-3 py-2 text-sm text-[var(--ui-danger)]"
				>
					<span>Lewdware failed to start: {launchError ?? engineStatus.error}</span>
				</div>
			{/if}
			{#if running && engineStatus.warning}
				<div
					class="flex items-center gap-3 rounded-md border border-[var(--ui-warning-border)] bg-[var(--ui-warning-bg)] px-3 py-2 text-sm text-[var(--ui-warning)]"
				>
					<span>{engineStatus.warning}</span>
				</div>
			{/if}
		</section>

		<!-- Panic Key -->
		<section class="border-border flex flex-col gap-2 border-t pt-6">
			<h2 class="ui-section-title">Panic key</h2>
			<p class="text-muted text-xs">Pressing this key combination closes the app immediately.</p>
			{#if !inputMonitoringGranted}
				<div
					class="flex flex-col gap-2 rounded-md border border-[var(--ui-warning-border)] bg-[var(--ui-warning-bg)] px-3 py-2 text-sm text-[var(--ui-warning)]"
				>
					<div class="flex items-center gap-3">
						<span>The panic key requires Input Monitoring permission.</span>
						<Button
							size="compact"
							variant="secondary"
							class="ml-auto"
							onclick={openInputMonitoringSettings}>Open Settings</Button
						>
					</div>
					{#if inputMonitoringPromptFailed}
						<p class="text-xs">
							The permission prompt could not be shown (the app may need to be signed). To enable
							manually: open <button
								onclick={() => api.openInputMonitoringSettings()}
								class="underline transition-colors hover:text-white"
								>System Settings → Privacy &amp; Security → Input Monitoring</button
							>
							and add Lewdware, then restart the app.
						</p>
					{/if}
				</div>
			{/if}
			<Card class="flex items-center justify-between gap-4 p-4">
				<div>
					<h3 class="text-text m-0 text-sm font-medium">Current shortcut</h3>
					<p class="text-muted m-0 mt-1 text-xs">
						Click the shortcut, then press the new key combination.
					</p>
				</div>
				<button
					type="button"
					aria-label={recording
						? 'Recording panic key'
						: `Change panic key, currently ${panicKeyDisplay}`}
					class="min-w-40 cursor-pointer rounded-md border-2 px-4 py-2 text-sm font-semibold transition-colors select-none
             {recording
						? 'bg-accent/10 border-accent text-accent-foreground'
						: 'bg-bg border-border text-text hover:border-border-strong'}"
					onclick={() => (recording = true)}
					onkeydown={handleKeyDown}
					onblur={() => (recording = false)}
				>
					{#if recording}<span class="font-normal italic">Press a key…</span>{:else}<kbd
							>{panicKeyDisplay}</kbd
						>{/if}
				</button>
			</Card>
		</section>

		<!-- Monitors -->
		<section class="border-border flex flex-col gap-2 border-t pt-6">
			<h2 class="ui-section-title">Monitors</h2>
			<p class="text-muted text-xs">Choose where popup media may appear.</p>
			{#if store.monitors.length > 0}
				<Card class="divide-border divide-y">
					{#each store.monitors as monitor (monitor.id)}
						<label
							class="hover:bg-surface-2 flex cursor-pointer items-center gap-3 px-4 py-3 transition-colors first:rounded-t-md last:rounded-b-md"
						>
							<Checkbox
								checked={!monitor.disabled}
								ariaLabel={monitor.name}
								onchange={(checked) => store.setMonitorEnabled(monitor.id, checked)}
							/>
							<span class="text-text min-w-0 flex-1 truncate text-sm">{monitor.name}</span>
							<span class="text-muted shrink-0 font-mono text-[11px]"
								>{monitor.width}×{monitor.height}</span
							>
							{#if monitor.primary}<span
									class="border-border bg-bg text-muted rounded-full border px-2 py-0.5 text-[10px] font-semibold"
									>Primary</span
								>{/if}
						</label>
					{/each}
				</Card>
			{:else}
				<Card class="border-dashed !border-[var(--ui-border-strong)] p-6 text-center"
					><p class="text-muted m-0 text-xs">No monitors were detected.</p></Card
				>
			{/if}
		</section>

		<!-- Logs -->
		<section class="border-border flex flex-col gap-2 border-t pt-6">
			<h2 class="ui-section-title">Diagnostics</h2>
			<p class="text-muted text-xs">
				Open the folder containing logs from Lewdware and its supporting apps.
			</p>
			<Button class="self-start" size="compact" onclick={openLogs}>Open logs folder</Button>
		</section>
	</div>
</div>
