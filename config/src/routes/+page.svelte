<script lang="ts">
	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { DocumentText, Icon } from 'svelte-hero-icons';
	import type { SupervisorStatusDto } from '$lib/types';
	import { store } from '$lib/store.svelte';
	import { api } from '$lib/api';
	import Behaviour from '$lib/Behaviour.svelte';
	import PackMode from '$lib/PackMode.svelte';
	import Scheduling from '$lib/Scheduling.svelte';
	import SessionControl from '$lib/SessionControl.svelte';
	import Tabs from '$ui/Tabs.svelte';
	import Button from '$ui/Button.svelte';
	import IconButton from '$ui/IconButton.svelte';
	import TaskStatus from '$lib/TaskStatus.svelte';
	import { taskFeedback } from '$lib/taskFeedback.svelte';

	onMount(() => {
		void store.load();
		void store.refreshSupervisorStatus();
		const unlisten = listen<SupervisorStatusDto>('supervisor:status', (event) => {
			store.applySupervisorStatus(event.payload);
		});
		return () => {
			unlisten.then((fn) => fn());
		};
	});

	const tabs = [
		{ id: 'pack_mode' as const, label: 'Pack & mode' },
		{ id: 'behaviour' as const, label: 'Behaviour' },
		{ id: 'scheduling' as const, label: 'Scheduling' }
	];

	async function openLogs() {
		taskFeedback.progress('logs', 'Opening logs folder…');
		try {
			await api.openLogs();
			taskFeedback.success('logs', 'Logs folder opened');
		} catch (err) {
			taskFeedback.error('logs', `Couldn’t open logs folder: ${String(err)}`);
		}
	}
</script>

<div class="bg-bg flex h-full min-h-0 font-sans">
	<!-- Sidebar -->
	<aside class="bg-surface border-border flex w-48 shrink-0 flex-col border-r">
		<div class="border-border flex h-16 items-center gap-1 border-b px-4">
			<div class="min-w-0 flex-1">
				<span class="text-text block text-sm font-semibold">Lewdware</span>
				<span class="text-muted block text-xs">Settings</span>
			</div>
			<IconButton label="Open logs folder" onclick={openLogs}>
				<span class="block h-4 w-4"><Icon src={DocumentText} /></span>
			</IconButton>
		</div>
		<nav class="p-3" aria-label="Settings sections">
			<Tabs
				{tabs}
				active={store.activeTab}
				orientation="vertical"
				onselect={(id) => (store.activeTab = id as typeof store.activeTab)}
			/>
		</nav>
		{#if store.ready}
			<SessionControl />
		{/if}
	</aside>

	<!-- Main content -->
	<main class="bg-bg flex flex-1 flex-col overflow-hidden">
		{#if store.loadError}
			<div class="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
				<div class="text-text text-sm font-semibold">Settings couldn’t be loaded</div>
				<p class="text-muted m-0 max-w-md text-xs">{store.loadError}</p>
				<Button variant="primary" loading={store.loading} onclick={() => store.load()}
					>Try again</Button
				>
			</div>
		{:else if !store.ready}
			<div class="flex flex-1 items-center justify-center">
				<p class="text-muted text-sm" role="status">Loading settings…</p>
			</div>
		{:else if store.activeTab === 'pack_mode'}
			<PackMode />
		{:else if store.activeTab === 'behaviour'}
			<Behaviour />
		{:else if store.activeTab === 'scheduling'}
			<Scheduling />
		{/if}
	</main>
	<TaskStatus />
</div>
