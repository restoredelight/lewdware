<script lang="ts">
	import { clampScroll } from '$ui/scroll';
	import { onMount, tick } from 'svelte';
	import { api } from './api';
	import { store } from './store.svelte';
	import { taskFeedback } from './taskFeedback.svelte';
	import { isFullRegion, type DiagnosticsDto, type LogLevel, type LogRecordDto } from './types';
	import Button from '$ui/Button.svelte';
	import Card from '$ui/Card.svelte';
	import Select from '$ui/Select.svelte';
	import Toggle from '$ui/Toggle.svelte';

	let diagnostics = $state<DiagnosticsDto | null>(null);
	let loading = $state(false);
	let loadError = $state<string | null>(null);
	let source = $state('all');
	let level = $state('all');
	let query = $state('');
	let followNewest = $state(true);
	let logViewport = $state<HTMLDivElement>();
	let lastFollowedTimestamp: string | null = null;

	const sourceLabels: Record<string, string> = {
		config: 'Settings',
		'pack-editor': 'Pack editor',
		'lewdware-supervisor': 'Supervisor',
		lewdware: 'Engine'
	};

	const sourceOptions = $derived([
		{ value: 'all', label: 'All components' },
		...[...new Set(diagnostics?.logs.map((record) => record.component) ?? [])].map((component) => ({
			value: component,
			label: sourceLabels[component] ?? component
		}))
	]);

	// `fields` is explicit in the current schema. Keep this fallback at the rendering boundary so
	// an incomplete record can never take down the whole diagnostics page.
	function recordFields(record: LogRecordDto): Record<string, unknown> {
		return record.fields ?? {};
	}

	const visibleLogs = $derived.by(() => {
		const needle = query.trim().toLocaleLowerCase();
		return (diagnostics?.logs ?? []).filter((record) => {
			if (source !== 'all' && record.component !== source) return false;
			if (level !== 'all' && record.level !== level) return false;
			if (!needle) return true;
			return [
				record.message,
				record.target,
				record.file ?? '',
				JSON.stringify(recordFields(record))
			]
				.join(' ')
				.toLocaleLowerCase()
				.includes(needle);
		});
	});

	// Keeping the DOM bounded matters more than retaining every row on screen. Copy diagnostics
	// still includes the full backend-bounded set, while the viewer shows the newest matching 500.
	const displayedLogs = $derived(visibleLogs.slice(-500));

	$effect(() => {
		const newestTimestamp = displayedLogs.at(-1)?.timestamp ?? null;
		if (!newestTimestamp && logViewport) {
			void tick().then(() => {
				if (logViewport) logViewport.scrollTop = 0;
			});
		}
		if (
			!followNewest ||
			!logViewport ||
			!newestTimestamp ||
			newestTimestamp === lastFollowedTimestamp
		)
			return;
		lastFollowedTimestamp = newestTimestamp;
		void tick().then(() => {
			if (logViewport) logViewport.scrollTop = logViewport.scrollHeight;
		});
	});

	function setFollowNewest(checked: boolean) {
		followNewest = checked;
		if (checked) lastFollowedTimestamp = null;
	}

	function handleLogScroll() {
		if (!logViewport) return;
		const distanceFromBottom =
			logViewport.scrollHeight - logViewport.scrollTop - logViewport.clientHeight;
		if (distanceFromBottom <= 8) {
			if (!followNewest) setFollowNewest(true);
		} else if (followNewest) {
			followNewest = false;
		}
	}

	onMount(() => {
		void refresh();
		const timer = window.setInterval(() => {
			if (followNewest) void refresh(true);
		}, 2_000);
		return () => window.clearInterval(timer);
	});

	async function refresh(silent = false) {
		if (!silent) loading = true;
		try {
			diagnostics = await api.getDiagnostics();
			loadError = null;
		} catch (err) {
			if (!silent) loadError = String(err);
		} finally {
			if (!silent) loading = false;
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

	async function copy(text: string, confirmation: string) {
		try {
			await navigator.clipboard.writeText(text);
			taskFeedback.confirm('clipboard', confirmation);
		} catch (err) {
			taskFeedback.error('clipboard', `Couldn’t copy to the clipboard: ${String(err)}`);
		}
	}

	function formatRecord(record: LogRecordDto): string {
		const source = sourceLabels[record.component] ?? record.component;
		const location = record.file
			? ` ${record.file}${record.line ? `:${record.line}` : ''}`
			: ` ${record.target}`;
		const fields = recordFields(record);
		const renderedFields = Object.keys(fields).length > 0 ? ` ${JSON.stringify(fields)}` : '';
		return `${record.timestamp} ${record.level.toUpperCase()} ${source}${location}: ${record.message}${renderedFields}`;
	}

	function selectedModeName(): string {
		return (
			store.modeGroups
				.flatMap((group) => group.entries)
				.find((mode) => store.isModeSelected(mode.id))?.name ?? 'Unknown'
		);
	}

	function packName(): string {
		return store.config?.pack_path?.split(/[\\/]/).filter(Boolean).at(-1) ?? 'None';
	}

	function diagnosticReport(): string {
		if (!diagnostics) return '';
		const system = diagnostics.system;
		const monitors = store.monitors.length
			? store.monitors
					.map((monitor) => {
						// The region belongs in a bug report: "popups only appear in one corner" is
						// indistinguishable from a placement bug without it.
						const region = isFullRegion(monitor.region)
							? ''
							: ` (area ${Math.round(monitor.region.width * 100)}%×${Math.round(monitor.region.height * 100)}%` +
								` at ${Math.round(monitor.region.x * 100)}%,${Math.round(monitor.region.y * 100)}%)`;

						return `- ${monitor.name}: ${monitor.width}×${monitor.height}${monitor.primary ? ' (primary)' : ''}${monitor.disabled ? ' (disabled)' : ''}${region}`;
					})
					.join('\n')
			: '- Unavailable';
		return [
			'### Lewdware diagnostics',
			'',
			`Generated: ${new Date().toISOString()}`,
			'',
			'#### System',
			'',
			`- Lewdware: ${system.lewdware_version}`,
			`- OS: ${system.os}`,
			`- Architecture: ${system.architecture}`,
			'',
			'#### Session',
			'',
			`- State: ${store.engineStatus.running ? 'Running' : 'Stopped'}`,
			`- Pack: ${packName()}`,
			`- Mode: ${selectedModeName()}`,
			`- Window style: ${store.config?.theme ?? 'Unknown'} (${store.config?.appearance ?? 'unknown'})`,
			'',
			'#### Monitors',
			'',
			monitors,
			'',
			`#### Recent logs (${diagnostics.logs.length})`,
			'',
			'````text',
			...diagnostics.logs.map(formatRecord),
			'````'
		].join('\n');
	}

	function levelClass(logLevel: LogLevel): string {
		if (logLevel === 'error') return 'text-[var(--ui-danger)]';
		if (logLevel === 'warn') return 'text-[var(--ui-warning)]';
		return 'text-muted';
	}

	function displayTime(timestamp: string): string {
		return new Date(timestamp).toLocaleTimeString([], {
			hour: '2-digit',
			minute: '2-digit',
			second: '2-digit'
		});
	}
</script>

<div class="min-h-0 flex-1 overflow-y-auto" use:clampScroll>
	<div class="mx-auto flex h-full min-h-[36rem] w-full max-w-5xl flex-col gap-6 p-8">
		<header class="flex items-start justify-between gap-6">
			<div class="max-w-2xl">
				<h1 class="ui-page-title">Diagnostics</h1>
				<p class="text-muted mt-1.5 mb-0 text-sm">
					Inspect recent activity and collect the information needed to investigate a problem.
				</p>
			</div>
			<div class="flex shrink-0 items-center gap-2">
				<Button variant="secondary" onclick={openLogs}>Open logs folder</Button>
				<Button
					variant="secondary"
					disabled={!diagnostics}
					onclick={() => copy(diagnosticReport(), 'Diagnostics copied')}>Copy diagnostics</Button
				>
			</div>
		</header>

		{#if loadError && !diagnostics}
			<Card
				class="flex flex-col items-center gap-2 border-dashed !border-[var(--ui-border-strong)] p-7 text-center"
			>
				<p class="text-text m-0 text-sm font-semibold">Diagnostics couldn’t be loaded</p>
				<p class="text-muted m-0 max-w-md text-xs">{loadError}</p>
				<Button class="mt-1" size="compact" {loading} onclick={() => refresh()}>Try again</Button>
			</Card>
		{:else}
			<section class="flex flex-col gap-2">
				<h2 class="ui-section-title">System information</h2>
				<Card class="divide-border grid grid-cols-3 divide-x">
					<div class="p-4">
						<p class="text-muted m-0 text-xs">Lewdware</p>
						<p class="text-text m-0 mt-0.5 font-mono text-[11.5px]">
							{diagnostics?.system.lewdware_version ?? 'Loading…'}
						</p>
					</div>
					<div class="min-w-0 p-4">
						<p class="text-muted m-0 text-xs">Operating system</p>
						<p
							class="text-text m-0 mt-0.5 truncate font-mono text-[11.5px]"
							title={diagnostics?.system.os}
						>
							{diagnostics?.system.os ?? 'Loading…'}
						</p>
					</div>
					<div class="p-4">
						<p class="text-muted m-0 text-xs">Architecture</p>
						<p class="text-text m-0 mt-0.5 font-mono text-[11.5px]">
							{diagnostics?.system.architecture ?? 'Loading…'}
						</p>
					</div>
				</Card>
			</section>

			<section class="border-border flex min-h-0 flex-1 flex-col gap-2 border-t pt-6">
				<div class="flex items-end justify-between gap-4">
					<div>
						<h2 class="ui-section-title">Recent logs</h2>
						<p class="text-muted mt-1 mb-0 text-xs">
							Logs are retained for up to 14 days and capped at 50 MB.
						</p>
					</div>
					<div class="flex items-center gap-3">
						<Toggle
							ariaLabel="Follow newest log entries"
							checked={followNewest}
							onchange={setFollowNewest}
						/>
						<span class="text-muted text-xs">Follow newest</span>
						<Button size="compact" {loading} onclick={() => refresh()}>Refresh</Button>
					</div>
				</div>

				<Card class="flex min-h-0 flex-1 flex-col">
					<div
						class="border-border bg-surface flex items-end gap-2 rounded-t-[var(--ui-radius-md)] border-b p-3"
					>
						<Select
							class="w-40"
							size="compact"
							hideLabel
							label="Component"
							value={source}
							options={sourceOptions}
							onchange={(value) => (source = value)}
						/>
						<Select
							class="w-32"
							size="compact"
							hideLabel
							label="Severity"
							value={level}
							options={[
								{ value: 'all', label: 'All levels' },
								{ value: 'error', label: 'Errors' },
								{ value: 'warn', label: 'Warnings' },
								{ value: 'info', label: 'Information' }
							]}
							onchange={(value) => (level = value)}
						/>
						<label class="min-w-0 flex-1">
							<span class="sr-only">Search logs</span>
							<input
								type="search"
								placeholder="Search logs"
								bind:value={query}
								class="border-border bg-bg text-text h-8 w-full rounded-sm border px-2.5 text-xs transition-colors outline-none placeholder:text-[var(--ui-muted)] hover:border-[var(--ui-border-strong)] focus:border-[var(--ui-focus)]"
							/>
						</label>
						<Button
							size="compact"
							disabled={visibleLogs.length === 0}
							onclick={() => copy(visibleLogs.map(formatRecord).join('\n'), 'Visible logs copied')}
							>Copy visible</Button
						>
					</div>

					<div
						class="bg-bg min-h-28 flex-1 overflow-auto"
						aria-live="polite"
						bind:this={logViewport}
						onscroll={handleLogScroll}
					>
						{#if loading && !diagnostics}
							<p
								class="text-muted m-0 flex min-h-28 items-center justify-center p-6 text-center text-xs"
								role="status"
							>
								Loading logs…
							</p>
						{:else if displayedLogs.length === 0}
							<p
								class="text-muted m-0 flex min-h-28 items-center justify-center p-6 text-center text-xs"
							>
								No matching log entries.
							</p>
						{:else}
							<div class="divide-border divide-y">
								{#each displayedLogs as record}
									<div
										class="grid grid-cols-[5.5rem_4rem_6.5rem_1fr] gap-2 px-3 py-2 font-mono text-[11px] leading-relaxed"
									>
										<span class="text-muted tabular-nums">{displayTime(record.timestamp)}</span>
										<span class={levelClass(record.level)}>{record.level}</span>
										<span class="text-muted truncate" title={record.component}>
											{sourceLabels[record.component] ?? record.component}
										</span>
										<div class="min-w-0">
											<p class="text-text m-0 break-words whitespace-pre-wrap">{record.message}</p>
										</div>
									</div>
								{/each}
							</div>
						{/if}
					</div>
					<div
						class="border-border bg-surface flex items-center justify-between rounded-b-[var(--ui-radius-md)] border-t px-3 py-2"
					>
						<p class="text-muted m-0 font-mono text-[10px]">
							Showing {displayedLogs.length} of {visibleLogs.length} matching entries
						</p>
						<p class="text-muted m-0 text-[10px]">
							Copied diagnostics may contain filenames or URLs. Review before sharing.
						</p>
					</div>
				</Card>
			</section>
		{/if}
	</div>
</div>
