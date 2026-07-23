<script lang="ts">
	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import type { SupervisorStatusDto } from '$lib/types';
	import { store } from '$lib/store.svelte';
	import General from '$lib/General.svelte';
	import PackMode from '$lib/PackMode.svelte';
	import Permissions from '$lib/Permissions.svelte';
	import Scheduling from '$lib/Scheduling.svelte';
	import Tabs from '$ui/Tabs.svelte';
	import Button from '$ui/Button.svelte';
	import TaskStatus from '$lib/TaskStatus.svelte';

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
		{ id: 'general' as const, label: 'General' },
		{ id: 'pack_mode' as const, label: 'Pack & Mode' },
		{ id: 'permissions' as const, label: 'Permissions & Volume' },
		{ id: 'scheduling' as const, label: 'Scheduling' }
	];
</script>

<div class="bg-bg flex h-full min-h-0 font-sans">
	<!-- Sidebar -->
	<aside class="bg-surface border-border flex w-48 shrink-0 flex-col border-r">
		<div class="border-border flex h-16 flex-col justify-center border-b px-4">
			<span class="text-text text-sm font-semibold">Lewdware</span>
			<span class="text-muted text-xs">Settings</span>
		</div>
		<nav class="p-3" aria-label="Settings sections">
			<Tabs
				{tabs}
				active={store.activeTab}
				orientation="vertical"
				onselect={(id) => (store.activeTab = id as typeof store.activeTab)}
			/>
		</nav>
		{#if store.engineStatus.running}
			<div
				class="border-border text-muted mt-auto flex items-center gap-2 border-t px-4 py-2.5 font-mono text-[11px]"
				role="status"
			>
				<span class="bg-accent h-1.5 w-1.5 shrink-0 rounded-full"></span>
				<span>running</span>
			</div>
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
		{:else if store.activeTab === 'general'}
			<General />
		{:else if store.activeTab === 'pack_mode'}
			<PackMode />
		{:else if store.activeTab === 'permissions'}
			<Permissions />
		{:else if store.activeTab === 'scheduling'}
			<Scheduling />
		{/if}
	</main>
	<TaskStatus />
</div>
