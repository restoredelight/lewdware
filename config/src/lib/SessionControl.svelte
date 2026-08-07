<script lang="ts">
	import { api } from './api';
	import { store } from './store.svelte';
	import { formatKey } from './keys';
	import Button from '$ui/Button.svelte';
	import { taskFeedback } from './taskFeedback.svelte';

	let engineAction = $state<'launch' | 'stop' | null>(null);

	const running = $derived(store.engineStatus.running);
	const hasPack = $derived(!!store.config?.pack_path);
	const panicKey = $derived(store.config ? formatKey(store.config.panic_button, '+') : null);

	// Engine-level failures -- a crash after a successful launch, the supervisor giving up on
	// restarts -- arrive on the pushed status rather than from the launch call, so they are routed
	// to the same feedback channel instead of being rendered a second way.
	$effect(() => {
		const status = store.engineStatus;
		if (!status.running && status.error) {
			taskFeedback.error('engine', `Lewdware stopped: ${status.error}`);
		} else if (status.running && status.warning) {
			taskFeedback.warning('engine', status.warning);
		}
	});

	async function launch() {
		engineAction = 'launch';
		taskFeedback.progress('engine', 'Launching Lewdware…');
		try {
			if (!(await store.saveConfig())) throw new Error('settings could not be saved');
			await api.launchLewdware();
			// The push subscription may take a beat to reach a freshly spawned supervisor.
			void store.refreshSupervisorStatus();
			taskFeedback.success('engine', 'Lewdware launched');
		} catch (err) {
			taskFeedback.error('engine', `Lewdware failed to launch: ${String(err)}`);
		} finally {
			engineAction = null;
		}
	}

	async function stop() {
		engineAction = 'stop';
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
</script>

<!-- Raised surface + strong border, so the one control that outlives every page detaches from the
     sidebar instead of banding across its foot. No `--ui-shadow-pop`: on a near-black ground the
     hard offset read as a smudge rather than depth, and ordinary surfaces seam rather than float --
     the pop stays exclusive to `Dialog`. The status dot lands where the motif's carmine titlebar
     dot would, which is as much of the spawned-window quotation as this needs. -->
<div
	class="mx-3 mt-auto mb-3 flex flex-col gap-2 rounded-[var(--ui-radius-md)] border border-[var(--ui-border-strong)] bg-[var(--ui-surface-raised)] p-3"
>
	<div class="text-muted flex items-center gap-2 font-mono text-[11px]" role="status">
		<span
			class="h-1.5 w-1.5 shrink-0 rounded-full {running
				? 'bg-accent'
				: 'bg-[var(--ui-border-strong)]'} {engineAction ? 'animate-pulse' : ''}"
		></span>
		<span>{running ? 'running' : 'stopped'}</span>
	</div>

	<!-- The panic key is the one thing worth reading at the moment you launch, so it is a readout
	     here rather than only a setting three clicks away. -->
	{#if !hasPack}
		<p class="text-muted m-0 truncate font-mono text-[11px]">select a pack first</p>
	{:else if panicKey}
		<p
			class="text-muted m-0 truncate font-mono text-[11px]"
			title={`Panic key: ${formatKey(store.config!.panic_button)}`}
		>
			panic {panicKey}
		</p>
	{/if}

	{#if running}
		<Button
			class="w-full"
			size="compact"
			variant="destructive"
			onclick={stop}
			loading={engineAction === 'stop'}>Stop</Button
		>
	{:else}
		<Button
			class="w-full"
			size="compact"
			variant="primary"
			onclick={launch}
			disabled={!hasPack}
			loading={engineAction === 'launch'}
			title={hasPack ? 'Launch with the selected pack and mode' : 'Select a pack first'}
			>Launch</Button
		>
	{/if}
</div>
